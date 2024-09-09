use aws_sdk_dynamodb::types::AttributeValue;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::fmt::Formatter;

pub type AttributeMap = HashMap<String, AttributeValue>;

#[derive(Debug, Clone, Serialize)]
pub enum Status {
    // TODO: add FilesUploaded(?)
    Pending,
    Compiling,
    Ready {
        presigned_urls: Vec<String>,
    },
    Failed(String),
}

impl Status {
    pub const fn attribute_name() -> &'static str {
        "Status"
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Status::Pending => write!(f, "Pending"),
            Status::Compiling => write!(f, "Compiling"),
            Status::Ready { .. } => write!(f, "Ready"),
            Status::Failed(msg) => write!(f, "Failed: {}", msg),
        }
    }
}

impl From<&Status> for u32 {
    fn from(value: &Status) -> Self {
        match value {
            Status::Pending => 0,
            Status::Compiling => 1,
            Status::Ready { .. } => 2,
            Status::Failed(_) => 3,
        }
    }
}

impl From<Status> for u32 {
    fn from(value: Status) -> Self {
        u32::from(&value)
    }
}

impl From<Status> for HashMap<String, AttributeValue> {
    fn from(value: Status) -> Self {
        match value.clone() {
            Status::Pending | Status::Compiling => HashMap::from([(
                Status::attribute_name().into(),
                AttributeValue::N(u32::from(&value).to_string()),
            )]),
            Status::Ready { presigned_urls } => HashMap::from([
                (
                    Status::attribute_name().into(),
                    AttributeValue::N(u32::from(&value).to_string()),
                ),
                (Item::data_attribute_name().into(), AttributeValue::Ss(presigned_urls)),
            ]),
            Status::Failed(val) => HashMap::from([
                (
                    Status::attribute_name().into(),
                    AttributeValue::N(u32::from(&value).to_string()),
                ),
                (Item::data_attribute_name().into(), AttributeValue::S(val)),
            ]),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ItemError {
    #[error("Invalid Item format")]
    FormatError,
    #[error(transparent)]
    ParseError(#[from] std::num::ParseIntError),
}

pub struct Item {
    // TODO: uuid?
    pub id: String,
    pub status: Status,
    // TODO: type: Compiling/Verifying
}

impl Item {
    pub const fn status_attribute_name() -> &'static str {
        Status::attribute_name()
    }

    pub const fn data_attribute_name() -> &'static str {
        "Data"
    }

    pub const fn id_attribute_name() -> &'static str {
        "ID"
    }

    pub const fn primary_key_name() -> &'static str {
        Self::id_attribute_name()
    }
}

impl From<Item> for AttributeMap {
    fn from(value: Item) -> Self {
        let mut item_map = HashMap::from([(Item::id_attribute_name().into(), AttributeValue::S(value.id))]);
        item_map.extend(HashMap::from(value.status));

        item_map
    }
}

impl TryFrom<&AttributeMap> for Status {
    type Error = ItemError;
    fn try_from(value: &AttributeMap) -> Result<Self, Self::Error> {
        let status = value.get(Status::attribute_name()).ok_or(ItemError::FormatError)?;
        let status: u32 = status
            .as_n()
            .map_err(|_| ItemError::FormatError)?
            .parse::<u32>()?;
        let status = match status {
            0 => Status::Pending,
            1 => Status::Compiling,
            2 => {
                let data = value.get(Item::data_attribute_name()).ok_or(ItemError::FormatError)?;
                let data = data.as_ss().map_err(|_| ItemError::FormatError)?;

                Status::Ready {
                    presigned_urls: data.clone(),
                }
            }
            3 => {
                let data = value.get(Item::data_attribute_name()).ok_or(ItemError::FormatError)?;
                let data = data.as_s().map_err(|_| ItemError::FormatError)?;

                Status::Failed(data.clone())
            }
            _ => return Err(ItemError::FormatError),
        };

        Ok(status)
    }
}

impl TryFrom<AttributeMap> for Item {
    type Error = ItemError;
    fn try_from(value: AttributeMap) -> Result<Item, Self::Error> {
        let id = value.get(Item::id_attribute_name()).ok_or(ItemError::FormatError)?;
        let id = id.as_s().map_err(|_| ItemError::FormatError)?;
        let status = (&value).try_into()?;

        Ok(Item {
            id: id.clone(),
            status,
        })
    }
}