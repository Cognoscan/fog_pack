use super::*;
use crate::error::{Error, Result};
use crate::{de::FogDeserializer, element::*, value::Value, value_ref::ValueRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::{default::Default, iter::repeat};

#[inline]
fn is_false(v: &bool) -> bool {
    !v
}
#[inline]
fn validator_is_any(v: &Validator) -> bool {
    *v == Validator::Any
}

#[inline]
fn u32_is_zero(v: &u32) -> bool {
    *v == 0
}

#[inline]
fn u32_is_max(v: &u32) -> bool {
    *v == u32::MAX
}

/// Validator for arrays.
///
/// This validator type will only pass array values. Validation passes if:
///
/// - If the `in` list is not empty, the array must be among the arrays in the list.
/// - The array must not be among the arrays in the `nin` list.
/// - The arrays's length is less than or equal to the value in `max_len`.
/// - The arrays's length is greater than or equal to the value in `min_len`.
/// - If `unique` is true, the array items are all unique.
/// - For each validator in the `contains` list, at least one item in the array passes.
/// - Each item in the array is checked with a validator at the same index in the `prefix` array.
///     All validators must pass. If there is no validator at the same index, the validator in
///     `items` must pass. If a validator is not used, it passes automatically.
/// - If `same_len` is not empty, the array indices it lists must all be null or
///   not present, or they must all be arrays that have the same lengths.
///
/// # Defaults
///
/// Fields that aren't specified for the validator use their defaults instead. The defaults for
/// each field are:
///
/// - comment: ""
/// - contains: empty
/// - items: Validator::Any
/// - prefix: empty
/// - max_len: u32::MAX
/// - min_len: u32::MIN
/// - in_list: empty
/// - nin_list: empty
/// - same_len: empty
/// - unique: false
/// - query: false
/// - array: false
/// - contains_ok: false
/// - unique_ok: false
/// - size: false
/// - same_len_ok: false
///
/// # Extensibility
///
/// Schemas can change over time. Even though a new scheme may have a different
/// hash, it might be compatible with the old schema. For arrays, this means
/// that any program's internal data structures can accommodate adding new
/// optional elements to the end of the array without issue. For an array
/// validator, this means allowing:
///
/// - `prefix` can append new new validators, provided that:
///     - `prefix` is not empty - this array validator must already be in use as
///       a list of specific items.
///     - If `items` has a validator, the new validators either match it, or are
///       themselves extensions of it (eg. extended map/array/enum validators).
///       If items does not have a validator, then this requirement is automatically met.
/// - `max_len` can be incremented
/// - `same_len` can include the indices of the new prefix validators.
/// - `comment` can be modified
///
/// There's not an obvious mapping for this on the Rust side.
/// [`serde_tuple`](https://crates.io/crates/serde_tuple) gets close, but
/// doesn't allow for unknown fields at the end, which is required for
/// extensibility. A future crate that does, or custom `Serialize` /
/// `Deserialize` implementations, would be able to. If said implementation is
/// able to capture additional unknown fields at the end, it must also not
/// serialize those, so that there isn't potential overlap with future extended
/// items.
///
/// # Query Checking
///
/// Queries for arrays are only allowed to use non-default values for each field if the
/// corresponding query permission is set in the schema's validator:
///
/// - query: `in` and `nin` lists
/// - array: `prefix` and `items`
/// - contains_ok: `contains`
/// - unique_ok: `unique`
/// - size: `max_len` and `min_len`
/// - same_len_ok: `same_len`
///
/// In addition, sub-validators in the query are matched against the schema's sub-validators:
///
/// - Each validator in `contains` is checked against all of the schema's `prefix` validators, as
///     well as its `items` validator.
/// - The `items` validator is checked against the schema's `items' validator
/// - The `prefix` validators are checked against the schema's `prefix` validators. Unmatched
///     query validators are checked against the schema's `items` validator.
///
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ArrayValidator {
    /// An optional comment explaining the validator.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
    /// For each validator in this array, at least one item in the array must pass the validator.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub contains: Vec<Validator>,
    /// A validator that each item in the array must pass, unless it is instead checked by
    /// `prefix`.
    #[serde(skip_serializing_if = "validator_is_any")]
    pub items: Box<Validator>,
    /// An array of validators, which are matched up against the items in the array. Unmatched
    /// validators automatically pass, while unmatched items are checked against the `items`
    /// Validator.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub prefix: Vec<Validator>,
    /// The maximum allowed number of items in the array.
    #[serde(skip_serializing_if = "u32_is_max")]
    pub max_len: u32,
    /// The minimum allowed number of items in the array.
    #[serde(skip_serializing_if = "u32_is_zero")]
    pub min_len: u32,
    /// A vector of specific allowed values, stored under the `in` field. If empty, this vector is not checked against.
    #[serde(rename = "in", skip_serializing_if = "Vec::is_empty")]
    pub in_list: Vec<Vec<Value>>,
    /// A vector of specific unallowed values, stored under the `nin` field.
    #[serde(rename = "nin", skip_serializing_if = "Vec::is_empty")]
    pub nin_list: Vec<Vec<Value>>,
    /// A list of which indices must either not be present or be null, or must
    /// all exist and have the same lengths.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub same_len: BTreeSet<usize>,
    /// If set, all items in the array must be unique.
    #[serde(skip_serializing_if = "is_false")]
    pub unique: bool,
    /// Indicates if the array is meant to be extensible.
    #[serde(skip_serializing_if = "is_false")]
    pub extend: bool,
    /// If true, queries against matching spots may have values in the `in` or `nin` lists.
    #[serde(skip_serializing_if = "is_false")]
    pub query: bool,
    /// If true, queries against matching spots may use `items` and `prefix`.
    #[serde(skip_serializing_if = "is_false")]
    pub array: bool,
    /// If true, queries against matching spots may use `contains`.
    #[serde(skip_serializing_if = "is_false")]
    pub contains_ok: bool,
    /// If true, queries against matching spots may use `unique`.
    #[serde(skip_serializing_if = "is_false")]
    pub unique_ok: bool,
    /// If true, queries against matching spots may use `max_len` and `min_len`.
    #[serde(skip_serializing_if = "is_false")]
    pub size: bool,
    /// If true, queries against matching spots may use `same_len`.
    #[serde(skip_serializing_if = "is_false")]
    pub same_len_ok: bool,
}

impl Default for ArrayValidator {
    fn default() -> Self {
        Self {
            comment: String::new(),
            contains: Vec::new(),
            items: Box::new(Validator::Any),
            prefix: Vec::new(),
            max_len: u32::MAX,
            min_len: u32::MIN,
            in_list: Vec::new(),
            nin_list: Vec::new(),
            same_len: BTreeSet::new(),
            extend: false,
            unique: false,
            query: false,
            array: false,
            contains_ok: false,
            unique_ok: false,
            size: false,
            same_len_ok: false,
        }
    }
}

impl ArrayValidator {
    /// Make a new validator with the default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a comment for the validator.
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = comment.into();
        self
    }

    /// Extend the `contains` list with another validator
    pub fn contains_add(mut self, validator: Validator) -> Self {
        self.contains.push(validator);
        self
    }

    /// Set the `items` validator.
    pub fn items(mut self, items: Validator) -> Self {
        self.items = Box::new(items);
        self
    }

    /// Extend the `prefix` list with another validator
    pub fn prefix_add(mut self, prefix: Validator) -> Self {
        self.prefix.push(prefix);
        self
    }

    /// Set the maximum number of allowed bytes.
    pub fn max_len(mut self, max_len: u32) -> Self {
        self.max_len = max_len;
        self
    }

    /// Set the minimum number of allowed bytes.
    pub fn min_len(mut self, min_len: u32) -> Self {
        self.min_len = min_len;
        self
    }

    /// Add a value to the `in` list.
    pub fn in_add(mut self, add: impl Into<Vec<Value>>) -> Self {
        self.in_list.push(add.into());
        self
    }

    /// Add a value to the `nin` list.
    pub fn nin_add(mut self, add: impl Into<Vec<Value>>) -> Self {
        self.nin_list.push(add.into());
        self
    }

    /// Add a key to the `same_len` list.
    pub fn same_len_add(mut self, add: usize) -> Self {
        self.same_len.insert(add);
        self
    }

    /// Mark whether or not the array can be extended.
    pub fn extensible(mut self, extend: bool) -> Self {
        self.extend = extend;
        self
    }

    /// Set whether the items in the array must be unique.
    pub fn unique(mut self, unique: bool) -> Self {
        self.unique = unique;
        self
    }

    /// Set whether or not queries can use the `in` and `nin` lists.
    pub fn query(mut self, query: bool) -> Self {
        self.query = query;
        self
    }

    /// Set whether or not queries can use the `items` and `prefix` values.
    pub fn array(mut self, array: bool) -> Self {
        self.array = array;
        self
    }

    /// Set whether or not queries can use the `contains` value.
    pub fn contains_ok(mut self, contains_ok: bool) -> Self {
        self.contains_ok = contains_ok;
        self
    }

    /// Set whether or not queries can use the `unique` setting.
    pub fn unique_ok(mut self, unique_ok: bool) -> Self {
        self.unique_ok = unique_ok;
        self
    }

    /// Set whether or not queries can use the `max_len` and `min_len` values.
    pub fn size(mut self, size: bool) -> Self {
        self.size = size;
        self
    }

    /// Set whether or not queries can use the `same_len` value.
    pub fn same_len_ok(mut self, same_len_ok: bool) -> Self {
        self.same_len_ok = same_len_ok;
        self
    }

    /// Build this into a [`Validator`] enum.
    pub fn build(self) -> Validator {
        Validator::Array(Box::new(self))
    }

    pub(crate) fn validate<'de, 'c>(
        &'c self,
        types: &'c BTreeMap<String, Validator>,
        mut parser: Parser<'de>,
        mut checklist: Option<Checklist<'c>>,
    ) -> Result<(Parser<'de>, Option<Checklist<'c>>)> {
        let val_parser = parser.clone();
        let elem = parser
            .next()
            .ok_or_else(|| Error::FailValidate("Expected an array".to_string()))??;
        let len = if let Element::Array(len) = elem {
            len
        } else {
            return Err(Error::FailValidate(format!(
                "Expected Array, got {}",
                elem.name()
            )));
        };

        if (len as u32) > self.max_len {
            return Err(Error::FailValidate(format!(
                "Array is {} elements, longer than maximum allowed of {}",
                len, self.max_len
            )));
        }
        if (len as u32) < self.min_len {
            return Err(Error::FailValidate(format!(
                "Array is {} elements, shorter than minimum allowed of {}",
                len, self.min_len
            )));
        }

        // Check all the requirements that require parsing the entire array
        if self.unique || !self.in_list.is_empty() || !self.nin_list.is_empty() {
            let mut de = FogDeserializer::from_parser(val_parser);
            let array = Vec::<ValueRef>::deserialize(&mut de)?;

            if !self.in_list.is_empty() && !self.in_list.iter().any(|v| *v == array) {
                return Err(Error::FailValidate("Array is not on `in` list".to_string()));
            }

            if self.nin_list.iter().any(|v| *v == array) {
                return Err(Error::FailValidate("Array is on `nin` list".to_string()));
            }

            if self.unique
                && array
                    .iter()
                    .enumerate()
                    .any(|(index, lhs)| array.iter().skip(index).any(|rhs| lhs == rhs))
            {
                return Err(Error::FailValidate(
                    "Array does not contain unique elements".to_string(),
                ));
            }
        }

        // Loop through each item, verifying it with the appropriate validator
        let mut contains_result = vec![false; self.contains.len()];
        let mut array_len: Option<usize> = None;
        let mut array_len_cnt = 0;
        let mut validators = self.prefix.iter().chain(repeat(self.items.as_ref()));
        for i in 0..len {
            // If we have a "contains", check and see if this item in the array
            // gets any of the "contains" validators to pass.
            if !self.contains.is_empty() {
                self.contains
                    .iter()
                    .zip(contains_result.iter_mut())
                    .for_each(|(validator, passed)| {
                        if !*passed {
                            let result =
                                validator.validate(types, parser.clone(), checklist.clone());
                            if let Ok((_, c)) = result {
                                *passed = true;
                                checklist = c;
                            }
                        }
                    });
            }

            // Check for same-length sub-arrays
            if self.same_len.contains(&i) {
                // Peek the array and its length
                let elem = parser.peek().ok_or_else(|| {
                    Error::FailValidate(format!("expected an array element at index {}", i))
                })??;
                match elem {
                    Element::Null => {
                        if array_len.is_some() {
                            return Err(Error::FailValidate(format!(
                                "some sub-arrays for `same_len` are present, but the one at {} is not",
                                i
                            )));
                        }
                    }
                    Element::Array(len) => {
                        if let Some(array_len) = array_len {
                            if array_len != len {
                                return Err(Error::FailValidate(format!(
                                    "expected array of length {} for index {}, but length was {}",
                                    array_len, i, len
                                )));
                            }
                        } else {
                            array_len = Some(len);
                        }
                        array_len_cnt += 1;
                    }
                    _ => {
                        return Err(Error::FailValidate(format!(
                            "`same_len` expected an array or null at index {}",
                            i
                        )))
                    }
                }
            }

            // Validate this item in the array against the next validator
            let (p, c) = validators
                .next()
                .unwrap()
                .validate(types, parser, checklist)?;
            parser = p;
            checklist = c;
        }

        if array_len.is_some() && array_len_cnt != self.same_len.len() {
            return Err(Error::FailValidate(
                "Array had some, but not all, of the indices listed in `same_len`".into(),
            ));
        }

        if !contains_result.iter().all(|x| *x) {
            let mut err_str = String::from("Array was missing items satisfying `contains` list:");
            let iter = contains_result
                .iter()
                .enumerate()
                .filter(|(_, pass)| !**pass)
                .map(|(index, _)| format!(" {},", index));
            err_str.extend(iter);
            err_str.pop(); // Remove the final comma
            return Err(Error::FailValidate(err_str));
        }
        Ok((parser, checklist))
    }

    fn query_check_self(
        &self,
        types: &BTreeMap<String, Validator>,
        other: &ArrayValidator,
    ) -> bool {
        let initial_check = (self.query || (other.in_list.is_empty() && other.nin_list.is_empty()))
            && (self.array || (other.prefix.is_empty() && validator_is_any(&other.items)))
            && (self.contains_ok || other.contains.is_empty())
            && (self.unique_ok || !other.unique)
            && (self.same_len_ok || other.same_len.is_empty())
            && (self.size || (u32_is_max(&other.max_len) && u32_is_zero(&other.min_len)));
        if !initial_check {
            return false;
        }
        if self.contains_ok {
            let contains_ok = other.contains.iter().all(|other| {
                self.items.query_check(types, other)
                    && self
                        .prefix
                        .iter()
                        .all(|mine| mine.query_check(types, other))
            });
            if !contains_ok {
                return false;
            }
        }
        if self.array {
            // Make sure item_type is OK, then check all items against their matching validator
            self.items.query_check(types, other.items.as_ref())
                && self
                    .prefix
                    .iter()
                    .chain(repeat(self.items.as_ref()))
                    .zip(other.prefix.iter().chain(repeat(other.items.as_ref())))
                    .take(self.prefix.len().max(other.prefix.len()))
                    .all(|(mine, other)| mine.query_check(types, other))
        } else {
            true
        }
    }

    pub(crate) fn query_check(
        &self,
        types: &BTreeMap<String, Validator>,
        other: &Validator,
    ) -> bool {
        match other {
            Validator::Array(other) => self.query_check_self(types, other),
            Validator::Multi(list) => list.iter().all(|other| match other {
                Validator::Array(other) => self.query_check_self(types, other),
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
    fn ser_default() {
        // Should be an empty map if we use the defaults
        let schema = ArrayValidator::default();
        let mut ser = FogSerializer::default();
        schema.serialize(&mut ser).unwrap();
        let expected: Vec<u8> = vec![0x80];
        let actual = ser.finish();
        println!("expected: {:x?}", expected);
        println!("actual:   {:x?}", actual);
        assert_eq!(expected, actual);

        let mut de = FogDeserializer::with_debug(&actual, "    ");
        let decoded = ArrayValidator::deserialize(&mut de).unwrap();
        println!("{}", de.get_debug().unwrap());
        assert_eq!(schema, decoded);
    }
}
