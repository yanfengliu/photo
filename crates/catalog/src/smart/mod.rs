//! Smart collection rules: `{ match_all, rules: [{ field, op, value }] }`,
//! stored as JSON and compiled to parameterised SQL (see `compile`).

mod compile;
#[cfg(test)]
mod format_tests;

use serde::{Deserialize, Serialize};

pub(crate) use compile::push_rules;

use crate::error::Result;
use crate::sql::Sql;

/// The rules of a smart collection. With no rules, every photo matches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmartRules {
    /// True if a photo must satisfy every rule, false if any one is enough.
    pub match_all: bool,
    /// The conditions, in the order the user listed them.
    pub rules: Vec<SmartRule>,
}

impl Default for SmartRules {
    /// No rules, combined with "all".
    fn default() -> Self {
        Self {
            match_all: true,
            rules: Vec::new(),
        }
    }
}

impl SmartRules {
    /// Version of the JSON shape rules are stored in: field names, nesting
    /// and the names of fields and operators. Stored beside every rule set;
    /// a build reads only rules of a format it knows, and never replaces
    /// rules of a newer one. Bump it whenever the shape changes; the test
    /// `the_rules_json_shape_is_pinned_to_its_format` fails until it is.
    pub const FORMAT: u32 = 1;

    /// The most rules one smart collection may combine. Each rule is one
    /// more nested condition and SQLite limits how deep conditions may nest.
    pub const MAX_RULES: usize = 100;

    /// Check that there are no more than [`SmartRules::MAX_RULES`] rules and
    /// that every rule pairs its field with an operator and a value that
    /// belong to it. A rule set that validates always compiles, and one that
    /// does not is never stored.
    pub fn validate(&self) -> Result<()> {
        push_rules(&mut Sql::default(), self)
    }
}

/// One condition on a photo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmartRule {
    /// What the rule looks at.
    pub field: SmartField,
    /// How it compares; must be one of the field's
    /// [`operators`](SmartField::operators).
    pub op: SmartOp,
    /// What to compare with, of the field's
    /// [`value_kind`](SmartField::value_kind); ignored by `IsEmpty` and
    /// `IsNotEmpty`.
    #[serde(default)]
    pub value: SmartValue,
}

/// What a rule looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmartField {
    /// The star rating, 0 to 5.
    Rating,
    /// The pick / reject flag.
    Flag,
    /// The colour label.
    Label,
    /// The names of the keywords assigned to the photo directly. A rule is
    /// about whether any one of them matches.
    Keyword,
    /// The title.
    Title,
    /// The caption.
    Caption,
    /// The creator.
    Creator,
    /// The copyright notice.
    Copyright,
    /// The camera's name; empty text where none is recorded.
    Camera,
    /// The lens's name; empty text where none is recorded.
    Lens,
    /// The calendar day of capture.
    CaptureDate,
    /// The ISO setting.
    Iso,
    /// The f-number.
    Aperture,
    /// Focal length in millimetres.
    FocalLength,
    /// Whether the file is RAW or raster.
    FileKind,
    /// Whether the develop settings differ from the default for the kind.
    Edited,
    /// The file's name, without its directory.
    FileName,
    /// The path of the folder that holds the file. `/` and `\` count as the
    /// same character, so a rule matches a Windows folder whichever slash
    /// it was typed with.
    FolderPath,
}

/// How a rule compares.
///
/// Where a photo has no value for a number or date field (no ISO recorded,
/// say), `IsNot` holds and every other comparison does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmartOp {
    /// Equal to the value (for text, ignoring case; for a date, on that day).
    Is,
    /// Not equal to the value.
    IsNot,
    /// Below the value (for a date, before that day).
    LessThan,
    /// The value or below.
    AtMost,
    /// Above the value (for a date, after that day).
    GreaterThan,
    /// The value or above.
    AtLeast,
    /// The text contains the value, ignoring case. An empty value is
    /// contained in everything.
    Contains,
    /// The text does not contain the value.
    DoesNotContain,
    /// The text begins with the value, ignoring case.
    StartsWith,
    /// The text ends with the value, ignoring case.
    EndsWith,
    /// The text is empty or only blanks: spaces, tabs and line breaks (for
    /// keywords: the photo has none).
    IsEmpty,
    /// The text is not empty (for keywords: the photo has at least one).
    IsNotEmpty,
}

/// The value a rule compares with.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SmartValue {
    /// No value, for operators that need none.
    #[default]
    None,
    /// True or false, for [`SmartValueKind::Boolean`] fields.
    Bool(bool),
    /// A finite number, for [`SmartValueKind::Number`] fields.
    Number(f64),
    /// Text, a date, or the name of a flag, label or file kind.
    Text(String),
}

impl From<bool> for SmartValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<f64> for SmartValue {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<i32> for SmartValue {
    fn from(value: i32) -> Self {
        Self::Number(f64::from(value))
    }
}

impl From<&str> for SmartValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<String> for SmartValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

/// The kind of value a field is compared with, for building a rule editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SmartValueKind {
    /// [`SmartValue::Number`].
    Number,
    /// [`SmartValue::Text`], matched ignoring case, every character literal.
    Text,
    /// [`SmartValue::Text`] holding a day as `YYYY-MM-DD`.
    Date,
    /// [`SmartValue::Text`]: `picked`, `unflagged` or `rejected`.
    Flag,
    /// [`SmartValue::Text`]: `none`, `red`, `yellow`, `green`, `blue` or
    /// `purple`.
    Label,
    /// [`SmartValue::Text`]: `raw` or `raster`.
    FileKind,
    /// [`SmartValue::Bool`].
    Boolean,
}

const COMPARISONS: &[SmartOp] = &[
    SmartOp::Is,
    SmartOp::IsNot,
    SmartOp::LessThan,
    SmartOp::AtMost,
    SmartOp::GreaterThan,
    SmartOp::AtLeast,
];
const IDENTITY: &[SmartOp] = &[SmartOp::Is, SmartOp::IsNot];
const TEXT_OPERATORS: &[SmartOp] = &[
    SmartOp::Contains,
    SmartOp::DoesNotContain,
    SmartOp::StartsWith,
    SmartOp::EndsWith,
    SmartOp::Is,
    SmartOp::IsNot,
    SmartOp::IsEmpty,
    SmartOp::IsNotEmpty,
];

impl SmartField {
    /// The kind of value this field is compared with.
    pub fn value_kind(self) -> SmartValueKind {
        match self {
            Self::Rating | Self::Iso | Self::Aperture | Self::FocalLength => SmartValueKind::Number,
            Self::Flag => SmartValueKind::Flag,
            Self::Label => SmartValueKind::Label,
            Self::FileKind => SmartValueKind::FileKind,
            Self::Edited => SmartValueKind::Boolean,
            Self::CaptureDate => SmartValueKind::Date,
            Self::Keyword
            | Self::Title
            | Self::Caption
            | Self::Creator
            | Self::Copyright
            | Self::Camera
            | Self::Lens
            | Self::FileName
            | Self::FolderPath => SmartValueKind::Text,
        }
    }

    /// The operators that are valid for this field, in the order a rule
    /// editor would offer them.
    pub fn operators(self) -> &'static [SmartOp] {
        match self.value_kind() {
            SmartValueKind::Number | SmartValueKind::Date => COMPARISONS,
            SmartValueKind::Text => TEXT_OPERATORS,
            SmartValueKind::Flag
            | SmartValueKind::Label
            | SmartValueKind::FileKind
            | SmartValueKind::Boolean => IDENTITY,
        }
    }
}
