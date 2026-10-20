//! `Tpl<T>`: a non-string config field that takes either a literal or a reference (`"{{trigger.finishOnMerge}}"`),
//! so a document validates before its references are filled in. After resolution the field is a literal.

use super::refs::has_ref;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq)]
pub enum Tpl<T> {
    Value(T),
    Ref(String),
}

impl<T> Tpl<T> {
    /// The literal (after resolution); a reference left unresolved has none.
    pub fn value(&self) -> Option<&T> {
        match self {
            Tpl::Value(v) => Some(v),
            Tpl::Ref(_) => None,
        }
    }
}

impl<T: Clone> Tpl<T> {
    pub fn or(&self, default: T) -> T {
        self.value().cloned().unwrap_or(default)
    }
}

impl<T: Default> Default for Tpl<T> {
    fn default() -> Self {
        Tpl::Value(T::default())
    }
}

impl<T: Serialize> Serialize for Tpl<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Tpl::Value(v) => v.serialize(s),
            Tpl::Ref(r) => s.serialize_str(r),
        }
    }
}

impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for Tpl<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        if let serde_json::Value::String(s) = &v
            && has_ref(s)
        {
            return Ok(Tpl::Ref(s.clone()));
        }
        // An unresolved reference resolves to null; a field that is null then takes its default.
        serde_json::from_value(v).map(Tpl::Value).map_err(serde::de::Error::custom)
    }
}

impl<T: JsonSchema> JsonSchema for Tpl<T> {
    fn schema_name() -> Cow<'static, str> {
        format!("Tpl_{}", T::schema_name()).into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let inner = generator.subschema_for::<T>();
        json_schema!({
            "anyOf": [inner, {"type": "string", "pattern": "\\{\\{.*\\}\\}", "description": "A reference"}]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_a_literal_or_a_reference() {
        let lit: Tpl<bool> = serde_json::from_value(serde_json::json!(true)).unwrap();
        assert_eq!(lit, Tpl::Value(true));
        let r: Tpl<bool> = serde_json::from_value(serde_json::json!("{{trigger.x}}")).unwrap();
        assert_eq!(r, Tpl::Ref("{{trigger.x}}".into()));
        assert!(!r.or(false));
        assert!(serde_json::from_value::<Tpl<bool>>(serde_json::json!("yes")).is_err());
        assert_eq!(serde_json::to_value(&r).unwrap(), serde_json::json!("{{trigger.x}}"));
    }
}
