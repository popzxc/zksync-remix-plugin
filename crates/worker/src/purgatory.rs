use std::collections::HashMap;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{interval, sleep};
use tokio::{sync::Mutex, task::JoinHandle};
use tracing::warn;
use types::item::{Status, TaskResult};
use types::ARTIFACTS_FOLDER;
use uuid::Uuid;

use crate::clients::dynamodb_client::DynamoDBClient;
use crate::clients::s3_client::S3Client;
use crate::utils::lib::{s3_artifacts_dir, s3_compilation_files_dir, timestamp};

pub type Timestamp = u64;

#[derive(Clone)]
pub struct Purgatory {
    inner: Arc<Mutex<Inner>>,
}

impl Purgatory {
    pub fn new(state: State, db_client: DynamoDBClient, s3_client: S3Client) -> Self {
        let mut handle = NonNull::dangling();
        let this = Self {
            inner: Arc::new(Mutex::new(Inner::new(handle, state, db_client, s3_client))),
        };

        {
            let mut inner = this.inner.try_lock().unwrap();
            let mut initialized_handle = tokio::spawn(this.clone().daemon());
            inner.handle = unsafe { NonNull::new_unchecked(&mut initialized_handle as *mut _) };
        }

        this
    }

    pub async fn purge(&mut self) {
        self.inner.lock().await.purge().await;
    }

    pub async fn add_record(&mut self, id: Uuid, result: TaskResult) {
        self.inner.lock().await.add_record(id, result);
    }

    async fn daemon(mut self) {
        const PURGE_INTERVAL: Duration = Duration::from_secs(60);

        let mut interval = interval(PURGE_INTERVAL);
        loop {
            interval.tick().await;
            self.purge().await;
        }
    }
}

pub struct State {
    expiration_timestamps: Vec<(Uuid, Timestamp)>,
    task_status: HashMap<Uuid, Status>,
}

impl State {
    pub async fn load() -> State {
        // TODO: load state here from DB
        Self {
            expiration_timestamps: vec![],
            task_status: HashMap::new(),
        }
    }
}

struct Inner {
    state: State,
    s3_client: S3Client,
    db_client: DynamoDBClient,

    // No aliases possible since only we own the data
    handle: NonNull<JoinHandle<()>>,
    _marker: PhantomData<JoinHandle<()>>,
}

unsafe impl Send for Inner {}

impl Drop for Inner {
    fn drop(&mut self) {
        unsafe {
            self.handle.as_ref().abort();
        }
    }
}

impl Inner {
    fn new(
        handle: NonNull<JoinHandle<()>>,
        state: State,
        db_client: DynamoDBClient,
        s3_client: S3Client,
    ) -> Self {
        Self {
            state,
            s3_client,
            db_client,
            handle,
            _marker: PhantomData,
        }
    }

    fn add_record(&mut self, id: Uuid, result: TaskResult) {
        pub const DURATION_TO_PURGE: u64 = 60 * 5; // 5 minutes
        let to_purge_timestampe = timestamp() + DURATION_TO_PURGE;

        self.state.task_status.insert(id, Status::Done(result));
        self.state
            .expiration_timestamps
            .push((id, to_purge_timestampe));
    }

    pub async fn purge(&mut self) {
        let now = timestamp();
        for (id, timestamp) in self.state.expiration_timestamps.iter() {
            if *timestamp > now {
                break;
            }

            let status = if let Some(status) = self.state.task_status.get(id) {
                status
            } else {
                warn!("Inconsistency! ID present vector but not in status map");
                continue;
            };

            match status {
                Status::InProgress => warn!("Item compiling for too long!"),
                Status::Pending => {
                    warn!("Item pending for too long");

                    // Remove compilation files
                    let files_dir = s3_compilation_files_dir(id.to_string().as_str());
                    self.s3_client.delete_dir(&files_dir).await.unwrap();

                    // Remove artifacts
                    let artifacts_dir = s3_compilation_files_dir(id.to_string().as_str());
                    self.s3_client.delete_dir(&artifacts_dir).await.unwrap(); // TODO: fix

                    // Second would give neater replies
                    self.db_client
                        .delete_item(id.to_string().as_str())
                        .await
                        .unwrap();
                }
                Status::Done(_) => {
                    let dir = s3_artifacts_dir(id.to_string().as_str());
                    self.s3_client.delete_dir(&dir).await.unwrap(); // TODO: fix
                    self.db_client
                        .delete_item(id.to_string().as_str())
                        .await
                        .unwrap();
                }
            }
        }

        self.state.expiration_timestamps.retain(|(id, timestamp)| {
            if *timestamp > now {
                return true;
            };

            self.state.task_status.remove(id);
            false
        });
    }
}
