//! Building typed values out of the tree.
//!
//! A serde deserializer over [`Node`], so a program can keep its `#[derive(Deserialize)]`
//! structs and still get everything this parser knows. What it adds over reading YAML the
//! usual way is the position: a value of the wrong shape is reported where it was written,
//! not at the top of the file.

use serde::de::value::StrDeserializer;
use serde::de::{self, IntoDeserializer, Visitor};
use serde::forward_to_deserialize_any;

use crate::error::Error;
use crate::node::{Entry, Node, Value};
use crate::span::Position;

pub fn from_str<T: serde::de::DeserializeOwned>(source: &str) -> Result<T, Error> {
    let document = crate::Document::parse(source)?;
    from_node(&document.root)
}

pub fn from_node<T: serde::de::DeserializeOwned>(node: &Node) -> Result<T, Error> {
    T::deserialize(node)
}

impl de::Error for Error {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self {
            message: message.to_string(),
            position: Position::default(),
        }
    }
}

impl Error {
    /// Keeps the first position set, which is the innermost one.
    fn at(mut self, position: Position) -> Self {
        if self.position == Position::default() {
            self.position = position;
        }
        self
    }
}

impl<'de> de::Deserializer<'de> for &Node {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        let result = match &self.value {
            Value::Null => visitor.visit_unit(),
            Value::Bool(value) => visitor.visit_bool(*value),
            Value::Int(value) => visitor.visit_i64(*value),
            Value::Float(value) => visitor.visit_f64(*value),
            Value::String(value) => visitor.visit_str(value),
            Value::Sequence(items) => visitor.visit_seq(Items {
                items: items.iter(),
            }),
            Value::Mapping(mapping) => visitor.visit_map(Pairs {
                entries: mapping.entries().iter(),
                value: None,
            }),
        };

        result.map_err(|error: Error| error.at(self.span.start))
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.value {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        let variant = match &self.value {
            // A bare name is a variant that carries nothing.
            Value::String(name) => Variant { name, value: None },
            // Anything else is written as the one-entry mapping `variant: value`.
            Value::Mapping(mapping) if mapping.len() == 1 => {
                let entry = &mapping.entries()[0];
                Variant {
                    name: entry
                        .key
                        .as_str()
                        .ok_or_else(|| de::Error::custom("a variant name must be a string"))?,
                    value: Some(&entry.value),
                }
            }
            _ => return Err(de::Error::custom("expected a name or a one-entry mapping")),
        };

        visitor
            .visit_enum(variant)
            .map_err(|error: Error| error.at(self.span.start))
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct seq tuple tuple_struct map struct
        identifier ignored_any
    }
}

struct Items<'a> {
    items: std::slice::Iter<'a, Node>,
}

impl<'de, 'a> de::SeqAccess<'de> for Items<'a> {
    type Error = Error;

    fn next_element_seed<T: de::DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        match self.items.next() {
            Some(item) => seed.deserialize(item).map(Some),
            None => Ok(None),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len())
    }
}

struct Pairs<'a> {
    entries: std::slice::Iter<'a, Entry>,
    value: Option<&'a Node>,
}

impl<'de, 'a> de::MapAccess<'de> for Pairs<'a> {
    type Error = Error;

    fn next_key_seed<K: de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        match self.entries.next() {
            Some(entry) => {
                self.value = Some(&entry.value);
                seed.deserialize(&entry.key).map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V: de::DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        let value = self
            .value
            .take()
            .ok_or_else(|| de::Error::custom("a key without a value"))?;

        seed.deserialize(value)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len())
    }
}

struct Variant<'a> {
    name: &'a str,
    value: Option<&'a Node>,
}

impl<'de, 'a> de::EnumAccess<'de> for Variant<'a> {
    type Error = Error;
    type Variant = Self;

    fn variant_seed<V: de::DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self), Error> {
        let name: StrDeserializer<Error> = self.name.into_deserializer();
        Ok((seed.deserialize(name)?, self))
    }
}

impl<'de, 'a> de::VariantAccess<'de> for Variant<'a> {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        Ok(())
    }

    fn newtype_variant_seed<T: de::DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
        seed.deserialize(self.carried()?)
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, Error> {
        de::Deserializer::deserialize_seq(self.carried()?, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        de::Deserializer::deserialize_map(self.carried()?, visitor)
    }
}

impl<'a> Variant<'a> {
    fn carried(&self) -> Result<&'a Node, Error> {
        self.value
            .ok_or_else(|| de::Error::custom(format!("`{}` expects a value", self.name)))
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "kebab-case", deny_unknown_fields)]
    struct Job {
        runs_on: String,
        #[serde(default)]
        needs: Vec<String>,
        #[serde(default)]
        timeout_minutes: Option<u32>,
        steps: Vec<Step>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Step {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        run: Option<String>,
    }

    #[test]
    fn builds_a_struct_out_of_a_document() {
        let job: Job = from_str(
            "runs-on: ubuntu-latest\nneeds: [one, two]\nsteps:\n  - run: echo hi\n  - name: second\n",
        )
        .unwrap();

        assert_eq!(job.runs_on, "ubuntu-latest");
        assert_eq!(job.needs, ["one", "two"]);
        assert_eq!(job.timeout_minutes, None);
        assert_eq!(job.steps.len(), 2);
        assert_eq!(job.steps[0].run.as_deref(), Some("echo hi"));
    }

    #[test]
    fn a_missing_field_is_reported_where_the_value_is() {
        let error = from_str::<Job>("steps: []\n").unwrap_err();

        assert!(error.message.contains("runs-on"), "{}", error.message);
    }

    #[test]
    fn a_field_of_the_wrong_shape_is_reported_where_it_was_written() {
        let error = from_str::<Job>("runs-on: ubuntu-latest\nsteps:\n  - run: [not, a, string]\n")
            .unwrap_err();

        assert_eq!(error.position.line, 2, "{error}");
    }

    #[test]
    fn an_unknown_field_is_refused() {
        let error = from_str::<Job>("runs-on: x\nsteps: []\nnonsense: 1\n").unwrap_err();

        assert!(error.message.contains("nonsense"), "{}", error.message);
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(untagged)]
    enum Scalars {
        One(String),
        Many(Vec<String>),
    }

    #[test]
    fn an_untagged_enum_picks_the_shape_that_fits() {
        assert_eq!(
            from_str::<Scalars>("just one\n").unwrap(),
            Scalars::One("just one".to_owned())
        );
        assert_eq!(
            from_str::<Scalars>("[a, b]\n").unwrap(),
            Scalars::Many(vec!["a".to_owned(), "b".to_owned()])
        );
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(tag = "using", rename_all = "kebab-case")]
    enum Runs {
        Composite { steps: Vec<Step> },
        Node24 { main: String },
    }

    #[test]
    fn an_internally_tagged_enum_reads_its_tag() {
        let runs: Runs = from_str("using: node24\nmain: index.js\n").unwrap();

        assert_eq!(
            runs,
            Runs::Node24 {
                main: "index.js".to_owned()
            }
        );
    }
}
