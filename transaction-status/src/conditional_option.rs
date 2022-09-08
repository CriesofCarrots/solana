use serde::ser::{Error, Serialize, Serializer};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConditionalOption<T: Default> {
    Some(T),
    None,
    Skip,
}

impl<T: Default> Default for ConditionalOption<T> {
    fn default() -> Self {
        Self::None
    }
}

impl<T: Default> ConditionalOption<T> {
    pub fn should_skip(&self) -> bool {
        match self {
            Self::Skip => true,
            _ => false,
        }
    }

    pub fn or_skip(option: Option<T>) -> Self {
        match option {
            Option::Some(item) => Self::Some(item),
            Option::None => Self::Skip,
        }
    }
}

impl<T: Default> From<Option<T>> for ConditionalOption<T> {
    fn from(option: Option<T>) -> Self {
        match option {
            Option::Some(item) => Self::Some(item),
            Option::None => Self::None,
        }
    }
}

pub fn serialize_default_if_none<S, T: Default + Serialize>(
    item: &ConditionalOption<T>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match item {
        ConditionalOption::Some(item) => item.serialize(serializer),
        ConditionalOption::None => T::default().serialize(serializer),
        ConditionalOption::Skip => Err(Error::custom("Skip variants should not be serialized")),
    }
}

pub fn serialize_null_if_none<S, T: Default + Serialize>(
    item: &ConditionalOption<T>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match item {
        ConditionalOption::Some(item) => item.serialize(serializer),
        ConditionalOption::None => serializer.serialize_none(),
        ConditionalOption::Skip => Err(Error::custom("Skip variants should not be serialized")),
    }
}
