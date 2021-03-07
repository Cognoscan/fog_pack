use super::*;
use crate::element::*;
use crate::Timestamp;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::default::Default;

#[inline]
fn is_false(v: &bool) -> bool {
    !v
}

const ZERO_TIME: Timestamp = Timestamp::zero();
const MIN_TIME: Timestamp = Timestamp::min_value();
const MAX_TIME: Timestamp = Timestamp::max_value();

#[inline]
fn time_is_zero(v: &Timestamp) -> bool {
    *v == ZERO_TIME
}

#[inline]
fn time_is_min(v: &Timestamp) -> bool {
    *v == MIN_TIME
}

#[inline]
fn time_is_max(v: &Timestamp) -> bool {
    *v == MAX_TIME
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TimeValidator {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(skip_serializing_if = "time_is_zero")]
    pub default: Timestamp,
    #[serde(skip_serializing_if = "time_is_max")]
    pub max: Timestamp,
    #[serde(skip_serializing_if = "time_is_min")]
    pub min: Timestamp,
    #[serde(skip_serializing_if = "is_false")]
    pub ex_max: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub ex_min: bool,
    #[serde(rename = "in", skip_serializing_if = "Vec::is_empty")]
    pub in_list: Vec<Timestamp>,
    #[serde(rename = "nin", skip_serializing_if = "Vec::is_empty")]
    pub nin_list: Vec<Timestamp>,
    #[serde(skip_serializing_if = "is_false")]
    pub query: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub ord: bool,
}

impl Default for TimeValidator {
    fn default() -> Self {
        Self {
            comment: String::new(),
            default: ZERO_TIME,
            max: MAX_TIME,
            min: MIN_TIME,
            ex_max: false,
            ex_min: false,
            in_list: Vec::new(),
            nin_list: Vec::new(),
            query: false,
            ord: false,
        }
    }
}

impl TimeValidator {
    pub(crate) fn validate(&self, parser: &mut Parser) -> Result<()> {
        let elem = parser
            .next()
            .ok_or(Error::FailValidate("Expected a timestamp".to_string()))??;
        let val = if let Element::Timestamp(v) = elem {
            v
        } else {
            return Err(Error::FailValidate(format!(
                "Expected Time, got {}",
                elem.name()
            )));
        };

        // Range checks
        let max_pass = if self.ex_max {
            val < self.max
        }
        else {
            val <= self.max
        };
        let min_pass = if self.ex_min {
            val > self.min
        }
        else {
            val >= self.min
        };
        if !max_pass {
            return Err(Error::FailValidate("Timestamp greater than maximum allowed".to_string()));
        }
        if !min_pass {
            return Err(Error::FailValidate("Timestamp less than minimum allowed".to_string()));
        }

        // in/nin checks
        if self.in_list.len() > 0 {
            if !self.in_list.iter().any(|v| *v == val) {
                return Err(Error::FailValidate(
                        "Timestamp is not on `in` list".to_string()
                ));
            }
        }
        if self.nin_list.iter().any(|v| *v == val) {
            return Err(Error::FailValidate("Timestamp is on `nin` list".to_string()));
        }

        Ok(())
    }

    fn query_check_self(&self, other: &Self) -> bool {
        (self.query || (other.in_list.is_empty() && other.nin_list.is_empty()))
            && (self.ord
                || (!other.ex_min
                    && !other.ex_max
                    && time_is_min(&other.min)
                    && time_is_max(&other.max)))
    }

    pub(crate) fn query_check(&self, other: &Validator) -> bool {
        match other {
            Validator::Time(other) => self.query_check_self(other),
            Validator::Multi(list) => list.iter().all(|other| match other {
                Validator::Time(other) => self.query_check_self(other),
                _ => false,
            }),
            Validator::Any => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{de::FogDeserializer, ser::FogSerializer};


    #[test]
    fn default_ser() {
        // Should be an empty map if we use the defaults
        let schema = TimeValidator::default();
        let mut ser = FogSerializer::default();
        schema.serialize(&mut ser).unwrap();
        let expected: Vec<u8> = vec![0x80];
        let actual = ser.finish();
        println!("expected: {:x?}", expected);
        println!("actual:   {:x?}", actual);
        assert_eq!(expected, actual);

        let mut de = FogDeserializer::new(&actual);
        let decoded = TimeValidator::deserialize(&mut de).unwrap();
        assert_eq!(schema, decoded);
    }

    #[test]
    fn example_ser() {
        let schema = TimeValidator {
            comment: "The year 2020".to_string(),
            default: Timestamp::from_utc(1577854800, 0).unwrap(),
            min: Timestamp::from_utc(1577854800, 0).unwrap(),
            max: Timestamp::from_utc(1609477200, 0).unwrap(),
            ex_min: false,
            ex_max: true,
            in_list: Vec::new(),
            nin_list: Vec::new(),
            query: true,
            ord: true,
        };
        let mut ser = FogSerializer::default();
        schema.serialize(&mut ser).unwrap();
        let mut expected: Vec<u8> = vec![0x87];
        serialize_elem(&mut expected, Element::Str("comment"));
        serialize_elem(&mut expected, Element::Str("The year 2020"));
        serialize_elem(&mut expected, Element::Str("default"));
        serialize_elem(&mut expected, Element::Timestamp(Timestamp::from_utc(1577854800, 0).unwrap()));
        serialize_elem(&mut expected, Element::Str("ex_max"));
        serialize_elem(&mut expected, Element::Bool(true));
        serialize_elem(&mut expected, Element::Str("max"));
        serialize_elem(&mut expected, Element::Timestamp(Timestamp::from_utc(1609477200, 0).unwrap()));
        serialize_elem(&mut expected, Element::Str("min"));
        serialize_elem(&mut expected, Element::Timestamp(Timestamp::from_utc(1577854800, 0).unwrap()));
        serialize_elem(&mut expected, Element::Str("ord"));
        serialize_elem(&mut expected, Element::Bool(true));
        serialize_elem(&mut expected, Element::Str("query"));
        serialize_elem(&mut expected, Element::Bool(true));
        let actual = ser.finish();
        println!("expected: {:x?}", expected);
        println!("actual:   {:x?}", actual);
        assert_eq!(expected, actual);

        let mut de = FogDeserializer::with_debug(&actual, "    ".to_string());
        match TimeValidator::deserialize(&mut de) {
            Ok(decoded) => assert_eq!(schema, decoded),
            Err(e) => {
                println!("{}", de.get_debug().unwrap());
                println!("Error: {}", e);
                panic!("Couldn't decode");
            }
        }
    }

}
