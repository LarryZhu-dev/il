use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;

struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("JSON with unique object keys")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<UniqueValue, E> { Ok(UniqueValue(Value::Bool(value))) }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<UniqueValue, E> { Ok(UniqueValue(value.into())) }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<UniqueValue, E> { Ok(UniqueValue(value.into())) }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<UniqueValue, E> {
                Number::from_f64(value).map(|n| UniqueValue(Value::Number(n))).ok_or_else(|| E::custom("nonfinite number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<UniqueValue, E> { Ok(UniqueValue(value.into())) }
            fn visit_string<E: de::Error>(self, value: String) -> Result<UniqueValue, E> { Ok(UniqueValue(value.into())) }
            fn visit_unit<E: de::Error>(self) -> Result<UniqueValue, E> { Ok(UniqueValue(Value::Null)) }
            fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<UniqueValue, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<UniqueValue>()? { values.push(value.0); }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<UniqueValue, A::Error> {
                let mut values = Map::new();
                while let Some(key) = object.next_key::<String>()? {
                    if values.contains_key(&key) { return Err(de::Error::custom(format!("duplicate JSON key: {key}"))); }
                    values.insert(key, object.next_value::<UniqueValue>()?.0);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub fn parse(text: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str::<UniqueValue>(text).map(|value| value.0)
}
