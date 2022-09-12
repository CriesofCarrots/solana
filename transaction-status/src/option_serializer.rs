use serde::{ser::Error, Deserialize, Deserializer, Serialize, Serializer};

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct OptionalValue<T> {
    #[serde(
        flatten,
        default = "OptionSerializer::skip",
        skip_serializing_if = "OptionSerializer::should_skip"
    )]
    value: OptionSerializer<T>,
}

impl<T> OptionalValue<T> {
    pub fn skip() -> Self {
        OptionSerializer::skip().into()
    }

    pub fn some(value: T) -> Self {
        OptionSerializer::Some(value).into()
    }

    pub fn unwrap_or_skip(option: Option<T>) -> Self {
        match option {
            Option::Some(item) => Self::some(item).into(),
            Option::None => Self::skip(),
        }
    }
}

impl<T> From<OptionSerializer<T>> for OptionalValue<T> {
    fn from(value: OptionSerializer<T>) -> Self {
        Self { value }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum OptionSerializer<T> {
    Some(T),
    None,
    Skip,
}

impl<T> Default for OptionSerializer<T> {
    fn default() -> Self {
        Self::None
    }
}

impl<T> OptionSerializer<T> {
    fn skip() -> Self {
        Self::Skip
    }

    fn should_skip(&self) -> bool {
        matches!(self, Self::Skip)
    }
}

impl<T> From<Option<T>> for OptionSerializer<T> {
    fn from(option: Option<T>) -> Self {
        match option {
            Option::Some(item) => Self::Some(item),
            Option::None => Self::None,
        }
    }
}

impl<T: Serialize> Serialize for OptionSerializer<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Some(item) => item.serialize(serializer),
            Self::None => serializer.serialize_none(),
            Self::Skip => Err(Error::custom("Skip variants should not be serialized")),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for OptionSerializer<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let option: Option<T> = Deserialize::deserialize(deserializer)?;
        Ok(option.into())
    }
}
