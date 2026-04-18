//! This module contains code to work with "versionspec". It represents the
//! version part of [`crate::MatchSpec`], e.g.: `>=3.4,<4.0`.

mod constraint;
pub(crate) mod parse;
pub(crate) mod version_tree;

use std::{
    borrow::Cow,
    fmt::{Display, Formatter},
    ops::Bound,
    str::FromStr,
};

use version_ranges::Ranges;

pub(crate) use constraint::is_start_of_version_constraint;
use constraint::Constraint;
pub use parse::ParseConstraintError;
use serde::{Deserialize, Serialize, Serializer};
use thiserror::Error;
use version_tree::VersionTree;

use crate::{
    version::{
        bump::{with_a_appended_to_last_plain_identifier, with_dev_on_last_segment},
        StrictVersion,
    },
    version_spec::version_tree::ParseVersionTreeError,
    ParseStrictness,
    ParseStrictness::Lenient,
    ParseVersionError, Version, VersionBumpError, VersionBumpType,
};

/// An operator to compare two versions.
#[allow(missing_docs)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub enum RangeOperator {
    Greater,
    GreaterEquals,
    Less,
    LessEquals,
}

impl RangeOperator {
    /// Returns the complement of the current operator.
    pub fn complement(self) -> Self {
        match self {
            RangeOperator::Greater => RangeOperator::LessEquals,
            RangeOperator::GreaterEquals => RangeOperator::Less,
            RangeOperator::Less => RangeOperator::GreaterEquals,
            RangeOperator::LessEquals => RangeOperator::Greater,
        }
    }
}

#[allow(missing_docs)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub enum StrictRangeOperator {
    StartsWith,
    NotStartsWith,
    Compatible,
    NotCompatible,
}

impl StrictRangeOperator {
    /// Returns the complement of the current operator.
    pub fn complement(self) -> Self {
        match self {
            StrictRangeOperator::StartsWith => StrictRangeOperator::NotStartsWith,
            StrictRangeOperator::NotStartsWith => StrictRangeOperator::StartsWith,
            StrictRangeOperator::Compatible => StrictRangeOperator::NotCompatible,
            StrictRangeOperator::NotCompatible => StrictRangeOperator::Compatible,
        }
    }
}

/// An operator set a version equal to another
#[allow(missing_docs)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub enum EqualityOperator {
    Equals,
    NotEquals,
}

impl EqualityOperator {
    /// Returns the complement of the current operator.
    pub fn complement(self) -> Self {
        match self {
            EqualityOperator::Equals => EqualityOperator::NotEquals,
            EqualityOperator::NotEquals => EqualityOperator::Equals,
        }
    }
}

/// Range and equality operators combined
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize)]
pub enum VersionOperators {
    /// Specifies a range of versions
    Range(RangeOperator),
    /// Specifies a range of versions using the strict operator
    StrictRange(StrictRangeOperator),
    /// Specifies an exact version
    Exact(EqualityOperator),
}

/// Logical operator used two compare groups of version comparisons. E.g.
/// `>=3.4,<4.0` or `>=3.4|<4.0`,
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub enum LogicalOperator {
    /// All comparators must evaluate to true for the group to evaluate to true.
    And,

    /// Any comparators must evaluate to true for the group to evaluate to true.
    Or,
}

impl LogicalOperator {
    /// Returns the complement of the operator.
    pub fn complement(self) -> Self {
        match self {
            LogicalOperator::And => LogicalOperator::Or,
            LogicalOperator::Or => LogicalOperator::And,
        }
    }
}

/// A version specification.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum VersionSpec {
    /// No version specified
    None,
    /// Any version
    Any,
    /// A version range
    Range(RangeOperator, Version),
    /// A version range using the strict operator
    StrictRange(StrictRangeOperator, StrictVersion),
    /// A exact version
    Exact(EqualityOperator, Version),
    /// A group of version specifications
    Group(LogicalOperator, Vec<VersionSpec>),
}

#[allow(clippy::enum_variant_names, missing_docs)]
#[derive(Debug, Clone, Eq, PartialEq, Error)]
pub enum ParseVersionSpecError {
    #[error(transparent)]
    InvalidVersion(#[from] ParseVersionError),

    #[error(transparent)]
    InvalidVersionTree(#[from] ParseVersionTreeError),

    #[error(transparent)]
    InvalidConstraint(#[from] ParseConstraintError),
}

impl From<Constraint> for VersionSpec {
    fn from(constraint: Constraint) -> Self {
        match constraint {
            Constraint::Any => VersionSpec::Any,
            Constraint::Comparison(op, ver) => VersionSpec::Range(op, ver),
            Constraint::StrictComparison(op, ver) => {
                VersionSpec::StrictRange(op, StrictVersion(ver))
            }
            Constraint::Exact(e, ver) => VersionSpec::Exact(e, ver),
        }
    }
}

impl FromStr for VersionSpec {
    type Err = ParseVersionSpecError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        VersionSpec::from_str(s, ParseStrictness::Lenient)
    }
}

impl VersionSpec {
    /// Parse a [`VersionSpec`] from a string.
    pub fn from_str(
        source: &str,
        strictness: ParseStrictness,
    ) -> Result<Self, ParseVersionSpecError> {
        fn parse_tree(
            tree: VersionTree<'_>,
            strictness: ParseStrictness,
        ) -> Result<VersionSpec, ParseVersionSpecError> {
            match tree {
                VersionTree::Term(str) => Ok(Constraint::from_str(str, strictness)
                    .map_err(ParseVersionSpecError::InvalidConstraint)?
                    .into()),
                VersionTree::Group(op, groups) => Ok(VersionSpec::Group(
                    op,
                    groups
                        .into_iter()
                        .map(|group| parse_tree(group, strictness))
                        .collect::<Result<_, ParseVersionSpecError>>()?,
                )),
            }
        }

        let version_tree =
            VersionTree::try_from(source).map_err(ParseVersionSpecError::InvalidVersionTree)?;

        parse_tree(version_tree, strictness)
    }
}

impl Display for VersionOperators {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionOperators::Range(r) => write!(f, "{r}"),
            VersionOperators::StrictRange(r) => write!(f, "{r}"),
            VersionOperators::Exact(r) => write!(f, "{r}"),
        }
    }
}

impl Display for RangeOperator {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RangeOperator::Greater => write!(f, ">"),
            RangeOperator::GreaterEquals => write!(f, ">="),
            RangeOperator::Less => write!(f, "<"),
            RangeOperator::LessEquals => write!(f, "<="),
        }
    }
}

impl Display for StrictRangeOperator {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            StrictRangeOperator::StartsWith => write!(f, "="),
            StrictRangeOperator::NotStartsWith => write!(f, "!=startswith"),
            StrictRangeOperator::Compatible => write!(f, "~="),
            StrictRangeOperator::NotCompatible => write!(f, "!~="),
        }
    }
}

impl Display for EqualityOperator {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Equals => write!(f, "=="),
            Self::NotEquals => write!(f, "!="),
        }
    }
}

impl Display for LogicalOperator {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            LogicalOperator::And => write!(f, ","),
            LogicalOperator::Or => write!(f, "|"),
        }
    }
}

impl Display for VersionSpec {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        fn write(
            spec: &VersionSpec,
            f: &mut Formatter<'_>,
            parent_op: Option<LogicalOperator>,
        ) -> std::fmt::Result {
            match spec {
                VersionSpec::Any => write!(f, "*"),
                VersionSpec::StrictRange(op, version) => match op {
                    StrictRangeOperator::StartsWith => write!(f, "{version}.*"),
                    StrictRangeOperator::NotStartsWith => write!(f, "!={version}.*"),
                    op => write!(f, "{op}{version}"),
                },
                VersionSpec::Range(op, version) => {
                    write!(f, "{op}{version}")
                }
                VersionSpec::Exact(op, version) => {
                    write!(f, "{op}{version}")
                }
                VersionSpec::Group(op, group) => {
                    let requires_parenthesis = matches!(
                        (op, parent_op),
                        (LogicalOperator::Or, Some(LogicalOperator::And))
                    );

                    if requires_parenthesis {
                        write!(f, "(")?;
                    }
                    for (i, spec) in group.iter().enumerate() {
                        if i > 0 {
                            write!(f, "{op}")?;
                        }
                        write(spec, f, Some(*op))?;
                    }
                    if requires_parenthesis {
                        write!(f, ")")?;
                    }
                    Ok(())
                }
                VersionSpec::None => write!(f, "!"),
            }
        }

        write(self, f, None)
    }
}

impl Serialize for VersionSpec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for VersionSpec {
    fn deserialize<D>(deserializer: D) -> Result<VersionSpec, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = Cow::<'de, str>::deserialize(deserializer)?;
        VersionSpec::from_str(&s, Lenient).map_err(serde::de::Error::custom)
    }
}

impl VersionSpec {
    /// Returns whether the version matches the specification.
    pub fn matches(&self, version: &Version) -> bool {
        match self {
            VersionSpec::None => false,
            VersionSpec::Any => true,
            VersionSpec::Exact(EqualityOperator::Equals, limit) => limit == version,
            VersionSpec::Exact(EqualityOperator::NotEquals, limit) => limit != version,
            VersionSpec::Range(RangeOperator::Greater, limit) => version > limit,
            VersionSpec::Range(RangeOperator::GreaterEquals, limit) => version >= limit,
            VersionSpec::Range(RangeOperator::Less, limit) => version < limit,
            VersionSpec::Range(RangeOperator::LessEquals, limit) => version <= limit,
            VersionSpec::StrictRange(StrictRangeOperator::StartsWith, limit) => {
                version.starts_with(&limit.0)
            }
            VersionSpec::StrictRange(StrictRangeOperator::NotStartsWith, limit) => {
                !version.starts_with(&limit.0)
            }
            VersionSpec::StrictRange(StrictRangeOperator::Compatible, limit) => {
                version.compatible_with(&limit.0)
            }
            VersionSpec::StrictRange(StrictRangeOperator::NotCompatible, limit) => {
                !version.compatible_with(&limit.0)
            }
            VersionSpec::Group(LogicalOperator::And, group) => {
                group.iter().all(|spec| spec.matches(version))
            }
            VersionSpec::Group(LogicalOperator::Or, group) => {
                group.iter().any(|spec| spec.matches(version))
            }
        }
    }
}

/// Returns the interval used to approximate `v.*` / `startswith(v)`.
///
/// For numeric-tail prefixes we use `dev` sentinels on the last segment, e.g.
/// `1.2.*` becomes `[1.2dev, 1.3dev)`. This matches the direct prefix logic
/// because `dev` sorts below identifiers and numerals, whereas `a0` excludes
/// values like `1.2dev`.
/// Plain identifier tails such as `1.2a` use an exact identifier ceiling,
/// e.g. `=1.2a` becomes `[1.2a, 1.2aa)`.
///
/// Caveat: prefixes can still have matching versions below their apparent lower
/// bound if a later segment introduces a `dev` component, e.g. `1.2a.dev`
/// starts with `1.2a` but sorts below it. We intentionally leave that family as
/// a documented best-effort limitation for now.
fn starts_with_range(v: &Version) -> Result<Ranges<Version>, VersionBumpError> {
    let lower = with_dev_on_last_segment(v).into_owned();
    let upper = prefix_upper_bound(v)?;
    Ok(Ranges::between(lower, upper))
}

fn prefix_upper_bound(v: &Version) -> Result<Version, VersionBumpError> {
    let upper = with_a_appended_to_last_plain_identifier(v);
    if upper.as_ref() != v {
        Ok(upper.into_owned())
    } else {
        Ok(with_dev_on_last_segment(&v.bump(VersionBumpType::Last)?).into_owned())
    }
}

/// Returns the range for all versions compatible with `v` (i.e. `~=v`).
///
/// For multi-segment versions this is `[v, prefix_upper_bound(v.pop_segments(1)))`.
/// For single-segment versions we need the next epoch boundary instead, so
/// `~=1` becomes `>=1,<1!0`.
///
/// As with [`starts_with_range`], non-numeric tail prefixes are only lowered on
/// a best-effort basis because a plain interval cannot always express their
/// exact lower or upper frontier.
fn compatible_with_range(v: &Version) -> Result<Ranges<Version>, VersionBumpError> {
    let lower = v.clone();
    let upper = if v.segment_count() == 1 {
        Version::from_str(&format!("{}!0", v.epoch() + 1))
            .expect("constructed epoch boundary is always a valid version")
    } else {
        let prefix = v
            .pop_segments(1)
            .expect("compatible version always has >= 2 segments");
        prefix_upper_bound(&prefix)?
    };
    Ok(Ranges::between(lower, upper))
}

impl TryFrom<&VersionSpec> for Ranges<Version> {
    type Error = VersionBumpError;

    fn try_from(spec: &VersionSpec) -> Result<Self, Self::Error> {
        match spec {
            VersionSpec::None => Ok(Ranges::empty()),
            VersionSpec::Any => Ok(Ranges::full()),
            VersionSpec::Exact(EqualityOperator::Equals, v) => Ok(Ranges::singleton(v.clone())),
            VersionSpec::Exact(EqualityOperator::NotEquals, v) => {
                Ok(Ranges::singleton(v.clone()).complement())
            }
            VersionSpec::Range(RangeOperator::Greater, v) => {
                Ok(Ranges::strictly_higher_than(v.clone()))
            }
            VersionSpec::Range(RangeOperator::GreaterEquals, v) => {
                Ok(Ranges::higher_than(v.clone()))
            }
            VersionSpec::Range(RangeOperator::Less, v) => {
                Ok(Ranges::strictly_lower_than(v.clone()))
            }
            VersionSpec::Range(RangeOperator::LessEquals, v) => Ok(Ranges::lower_than(v.clone())),
            VersionSpec::StrictRange(StrictRangeOperator::StartsWith, v) => starts_with_range(&v.0),
            VersionSpec::StrictRange(StrictRangeOperator::NotStartsWith, v) => {
                Ok(starts_with_range(&v.0)?.complement())
            }
            VersionSpec::StrictRange(StrictRangeOperator::Compatible, v) => {
                compatible_with_range(&v.0)
            }
            VersionSpec::StrictRange(StrictRangeOperator::NotCompatible, v) => {
                Ok(compatible_with_range(&v.0)?.complement())
            }
            VersionSpec::Group(LogicalOperator::And, specs) => {
                specs.iter().try_fold(Ranges::full(), |acc, s| {
                    let r = Ranges::try_from(s)?;
                    Ok(acc.intersection(&r))
                })
            }
            VersionSpec::Group(LogicalOperator::Or, specs) => {
                specs.iter().try_fold(Ranges::empty(), |acc, s| {
                    let r = Ranges::try_from(s)?;
                    Ok(acc.union(&r))
                })
            }
        }
    }
}

/// Converts a set of version ranges back into a [`VersionSpec`].
///
/// Note: this conversion is lossy for specs that use strict operators
/// (e.g. `~=2.4`, `1.2.*`, `!=1.0`). Those are converted to their
/// equivalent simple range form (e.g. `>=2.4,<3.0a0`), so a round-trip
/// through [`Ranges`] preserves *semantics* but not the original syntax.
impl From<Ranges<Version>> for VersionSpec {
    fn from(ranges: Ranges<Version>) -> Self {
        fn segment_to_spec(lower: Bound<Version>, upper: Bound<Version>) -> VersionSpec {
            use std::ops::Bound::{Excluded, Included, Unbounded};

            match (lower, upper) {
                (Unbounded, Unbounded) => VersionSpec::Any,
                (Unbounded, Excluded(v)) => VersionSpec::Range(RangeOperator::Less, v),
                (Unbounded, Included(v)) => VersionSpec::Range(RangeOperator::LessEquals, v),
                (Included(v), Unbounded) => VersionSpec::Range(RangeOperator::GreaterEquals, v),
                (Excluded(v), Unbounded) => VersionSpec::Range(RangeOperator::Greater, v),
                (Included(lo), Included(hi)) if lo == hi => {
                    VersionSpec::Exact(EqualityOperator::Equals, lo)
                }
                (Included(lo), Excluded(hi)) => VersionSpec::Group(
                    LogicalOperator::And,
                    vec![
                        VersionSpec::Range(RangeOperator::GreaterEquals, lo),
                        VersionSpec::Range(RangeOperator::Less, hi),
                    ],
                ),
                (Included(lo), Included(hi)) => VersionSpec::Group(
                    LogicalOperator::And,
                    vec![
                        VersionSpec::Range(RangeOperator::GreaterEquals, lo),
                        VersionSpec::Range(RangeOperator::LessEquals, hi),
                    ],
                ),
                (Excluded(lo), Excluded(hi)) => VersionSpec::Group(
                    LogicalOperator::And,
                    vec![
                        VersionSpec::Range(RangeOperator::Greater, lo),
                        VersionSpec::Range(RangeOperator::Less, hi),
                    ],
                ),
                (Excluded(lo), Included(hi)) => VersionSpec::Group(
                    LogicalOperator::And,
                    vec![
                        VersionSpec::Range(RangeOperator::Greater, lo),
                        VersionSpec::Range(RangeOperator::LessEquals, hi),
                    ],
                ),
            }
        }

        let mut specs: Vec<VersionSpec> = ranges
            .into_iter()
            .map(|(lower, upper)| segment_to_spec(lower, upper))
            .collect();

        match specs.len() {
            0 => VersionSpec::None,
            1 => specs.pop().expect("a single segment exists"),
            _ => VersionSpec::Group(LogicalOperator::Or, specs),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use assert_matches::assert_matches;
    use rstest::rstest;

    use version_ranges::Ranges;

    use crate::{
        version_spec::{
            parse::ParseConstraintError, EqualityOperator, LogicalOperator, ParseVersionSpecError,
            RangeOperator, StrictRangeOperator,
        },
        ParseStrictness, StrictVersion, Version, VersionSpec,
    };

    fn assert_same_matches(lhs: &VersionSpec, rhs: &VersionSpec, versions: &[&str]) {
        for version_str in versions {
            let version = Version::from_str(version_str).unwrap();
            assert_eq!(
                lhs.matches(&version),
                rhs.matches(&version),
                "mismatch for version={version_str}, lhs={lhs}, rhs={rhs}"
            );
        }
    }

    #[test]
    fn test_simple() {
        assert_eq!(
            VersionSpec::from_str("==1.2.3", ParseStrictness::Strict),
            Ok(VersionSpec::Exact(
                EqualityOperator::Equals,
                Version::from_str("1.2.3").unwrap(),
            ))
        );
        assert_eq!(
            VersionSpec::from_str(">=1.2.3", ParseStrictness::Strict),
            Ok(VersionSpec::Range(
                RangeOperator::GreaterEquals,
                Version::from_str("1.2.3").unwrap(),
            ))
        );
        assert_eq!(
            VersionSpec::from_str("=1.2.3", ParseStrictness::Strict),
            Ok(VersionSpec::StrictRange(
                StrictRangeOperator::StartsWith,
                StrictVersion::from_str("1.2.3").unwrap(),
            ))
        );
    }

    #[test]
    fn test_group() {
        assert_eq!(
            VersionSpec::from_str(">=1.2.3,<2.0.0", ParseStrictness::Strict),
            Ok(VersionSpec::Group(
                LogicalOperator::And,
                vec![
                    VersionSpec::Range(
                        RangeOperator::GreaterEquals,
                        Version::from_str("1.2.3").unwrap(),
                    ),
                    VersionSpec::Range(RangeOperator::Less, Version::from_str("2.0.0").unwrap()),
                ],
            ))
        );
        assert_eq!(
            VersionSpec::from_str(">=1.2.3|<1.0.0", ParseStrictness::Strict),
            Ok(VersionSpec::Group(
                LogicalOperator::Or,
                vec![
                    VersionSpec::Range(
                        RangeOperator::GreaterEquals,
                        Version::from_str("1.2.3").unwrap(),
                    ),
                    VersionSpec::Range(RangeOperator::Less, Version::from_str("1.0.0").unwrap()),
                ],
            ))
        );
        assert_eq!(
            VersionSpec::from_str("((>=1.2.3)|<1.0.0)", ParseStrictness::Strict),
            Ok(VersionSpec::Group(
                LogicalOperator::Or,
                vec![
                    VersionSpec::Range(
                        RangeOperator::GreaterEquals,
                        Version::from_str("1.2.3").unwrap(),
                    ),
                    VersionSpec::Range(RangeOperator::Less, Version::from_str("1.0.0").unwrap()),
                ],
            ))
        );
    }

    #[test]
    fn test_matches() {
        let v1 = Version::from_str("1.2.0").unwrap();
        let vs1 = VersionSpec::from_str(">=1.2.3,<2.0.0", ParseStrictness::Strict).unwrap();
        assert!(!vs1.matches(&v1));

        let vs2 = VersionSpec::from_str("==1.2.0", ParseStrictness::Strict).unwrap();
        assert!(vs2.matches(&v1));

        let v2 = Version::from_str("1.2.3").unwrap();
        assert!(vs1.matches(&v2));
        assert!(!vs2.matches(&v2));

        let v3 = Version::from_str("1!1.2.3").unwrap();

        assert!(!vs1.matches(&v3));
        assert!(!vs2.matches(&v3));

        let vs3 = VersionSpec::from_str(">=1!1.2,<1!2", ParseStrictness::Strict).unwrap();
        assert!(vs3.matches(&v3));

        let vs4 = VersionSpec::from_str("1!1.2.*", ParseStrictness::Strict).unwrap();
        assert!(vs4.matches(&v3));
    }

    #[test]
    fn issue_204() {
        assert!(VersionSpec::from_str(">=3.8<3.9", ParseStrictness::Strict).is_err());
    }

    #[rstest]
    #[case("2.38.*", true)]
    #[case("2.38.0.*", true)]
    #[case("2.38.0.1*", false)]
    #[case("2.38.0a.*", false)]
    fn issue_685(#[case] spec: &str, #[case] starts_with: bool) {
        let spec = VersionSpec::from_str(spec, ParseStrictness::Strict).unwrap();
        let version = &Version::from_str("2.38").unwrap();
        assert_eq!(spec.matches(version), starts_with);
    }

    #[test]
    fn issue_225() {
        let spec = VersionSpec::from_str("~=2.4", ParseStrictness::Strict).unwrap();
        assert!(!spec.matches(&Version::from_str("3.1").unwrap()));
        assert!(spec.matches(&Version::from_str("2.4").unwrap()));
        assert!(spec.matches(&Version::from_str("2.5").unwrap()));
        assert!(!spec.matches(&Version::from_str("2.1").unwrap()));
    }

    #[test]
    fn issue_235() {
        assert_eq!(
            VersionSpec::from_str(">2.10*", ParseStrictness::Lenient).unwrap(),
            VersionSpec::from_str(">=2.10", ParseStrictness::Strict).unwrap()
        );
    }

    #[test]
    fn issue_mkl_double() {
        assert_eq!(
            VersionSpec::from_str("2023.*.*", ParseStrictness::Lenient).unwrap(),
            VersionSpec::from_str("2023.*", ParseStrictness::Lenient).unwrap()
        );
        assert!(VersionSpec::from_str("2023.*.*", ParseStrictness::Strict).is_err());
        assert_matches!(
            VersionSpec::from_str("2023.*.0", ParseStrictness::Lenient).unwrap_err(),
            ParseVersionSpecError::InvalidConstraint(
                ParseConstraintError::RegexConstraintsNotSupported
            )
        );
    }

    #[test]
    fn issue_722() {
        assert_eq!(
            VersionSpec::from_str("0.2.18.*.", ParseStrictness::Lenient).unwrap(),
            VersionSpec::from_str("0.2.18.*", ParseStrictness::Lenient).unwrap()
        );

        assert!(VersionSpec::from_str("0.2.18.*.", ParseStrictness::Strict).is_err());
    }

    #[test]
    fn issue_1004() {
        assert_eq!(
            VersionSpec::from_str(">=2.*.*", ParseStrictness::Lenient).unwrap(),
            VersionSpec::from_str(">=2", ParseStrictness::Lenient).unwrap()
        );

        assert!(VersionSpec::from_str("0.2.18.*.*", ParseStrictness::Strict).is_err());
    }

    #[test]
    fn issue_bracket_printing() {
        let v = VersionSpec::from_str("(>=1,<2)|>3", ParseStrictness::Lenient).unwrap();
        assert_eq!(format!("{v}"), ">=1,<2|>3");

        let v = VersionSpec::from_str("(>=1|<2),>3", ParseStrictness::Lenient).unwrap();
        assert_eq!(format!("{v}"), "(>=1|<2),>3");

        let v = VersionSpec::from_str("(>=1|<2)|>3", ParseStrictness::Lenient).unwrap();
        assert_eq!(format!("{v}"), ">=1|<2|>3");

        let v = VersionSpec::from_str("(>=1,<2),>3", ParseStrictness::Lenient).unwrap();
        assert_eq!(format!("{v}"), ">=1,<2,>3");

        let v =
            VersionSpec::from_str("((>=1|>2),(>3|>4))|(>5,<6)", ParseStrictness::Lenient).unwrap();
        assert_eq!(format!("{v}"), "(>=1|>2),(>3|>4)|>5,<6");
    }

    #[test]
    fn issue_star_operator() {
        assert_eq!(
            VersionSpec::from_str(">=*", ParseStrictness::Lenient).unwrap(),
            VersionSpec::from_str("*", ParseStrictness::Lenient).unwrap()
        );
        assert_eq!(
            VersionSpec::from_str("==*", ParseStrictness::Lenient).unwrap(),
            VersionSpec::from_str("*", ParseStrictness::Lenient).unwrap()
        );
        assert_eq!(
            VersionSpec::from_str("=*", ParseStrictness::Lenient).unwrap(),
            VersionSpec::from_str("*", ParseStrictness::Lenient).unwrap()
        );
        assert_eq!(
            VersionSpec::from_str("~=*", ParseStrictness::Lenient).unwrap(),
            VersionSpec::from_str("*", ParseStrictness::Lenient).unwrap()
        );
        assert_eq!(
            VersionSpec::from_str("<=*", ParseStrictness::Lenient).unwrap(),
            VersionSpec::from_str("*", ParseStrictness::Lenient).unwrap()
        );

        assert_matches!(
            VersionSpec::from_str(">*", ParseStrictness::Lenient).unwrap_err(),
            ParseVersionSpecError::InvalidConstraint(
                ParseConstraintError::GlobVersionIncompatibleWithOperator(_)
            )
        );
        assert_matches!(
            VersionSpec::from_str("!=*", ParseStrictness::Lenient).unwrap_err(),
            ParseVersionSpecError::InvalidConstraint(
                ParseConstraintError::GlobVersionIncompatibleWithOperator(_)
            )
        );
        assert_matches!(
            VersionSpec::from_str("<*", ParseStrictness::Lenient).unwrap_err(),
            ParseVersionSpecError::InvalidConstraint(
                ParseConstraintError::GlobVersionIncompatibleWithOperator(_)
            )
        );

        assert_matches!(
            VersionSpec::from_str(">=*", ParseStrictness::Strict).unwrap_err(),
            ParseVersionSpecError::InvalidConstraint(
                ParseConstraintError::GlobVersionIncompatibleWithOperator(_)
            )
        );
        assert_matches!(
            VersionSpec::from_str("==*", ParseStrictness::Strict).unwrap_err(),
            ParseVersionSpecError::InvalidConstraint(
                ParseConstraintError::GlobVersionIncompatibleWithOperator(_)
            )
        );
        assert_matches!(
            VersionSpec::from_str("=*", ParseStrictness::Strict).unwrap_err(),
            ParseVersionSpecError::InvalidConstraint(
                ParseConstraintError::GlobVersionIncompatibleWithOperator(_)
            )
        );
        assert_matches!(
            VersionSpec::from_str("~=*", ParseStrictness::Strict).unwrap_err(),
            ParseVersionSpecError::InvalidConstraint(
                ParseConstraintError::GlobVersionIncompatibleWithOperator(_)
            )
        );
        assert_matches!(
            VersionSpec::from_str("<=*", ParseStrictness::Strict).unwrap_err(),
            ParseVersionSpecError::InvalidConstraint(
                ParseConstraintError::GlobVersionIncompatibleWithOperator(_)
            )
        );
    }

    #[test]
    fn test_try_from_none() {
        let ranges = Ranges::<Version>::try_from(&VersionSpec::None).unwrap();
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_try_from_any() {
        let ranges = Ranges::<Version>::try_from(&VersionSpec::Any).unwrap();
        assert_eq!(ranges, Ranges::full());
    }

    #[test]
    fn test_try_from_exact_equals() {
        let spec = VersionSpec::from_str("==1.2.3", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert!(ranges.contains(&Version::from_str("1.2.3").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1.2.4").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1.2.2").unwrap()));
    }

    #[test]
    fn test_try_from_exact_not_equals() {
        let spec = VersionSpec::from_str("!=1.2.3", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert!(!ranges.contains(&Version::from_str("1.2.3").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.2.2").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.2.4").unwrap()));
    }

    #[test]
    fn test_try_from_greater_than() {
        let ranges = Ranges::<Version>::try_from(
            &VersionSpec::from_str(">1.2.3", ParseStrictness::Strict).unwrap(),
        )
        .unwrap();
        assert!(!ranges.contains(&Version::from_str("1.0").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1.2.3").unwrap()));
        assert!(ranges.contains(&Version::from_str("2.0").unwrap()));
    }

    #[test]
    fn test_try_from_greater_equals() {
        let ranges = Ranges::<Version>::try_from(
            &VersionSpec::from_str(">=1.2.3", ParseStrictness::Strict).unwrap(),
        )
        .unwrap();
        assert!(!ranges.contains(&Version::from_str("1.0").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.2.3").unwrap()));
        assert!(ranges.contains(&Version::from_str("2.0").unwrap()));
    }

    #[test]
    fn test_try_from_less_than() {
        let ranges = Ranges::<Version>::try_from(
            &VersionSpec::from_str("<1.2.3", ParseStrictness::Strict).unwrap(),
        )
        .unwrap();
        assert!(ranges.contains(&Version::from_str("1.0").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1.2.3").unwrap()));
        assert!(!ranges.contains(&Version::from_str("2.0").unwrap()));
    }

    #[test]
    fn test_try_from_less_equals() {
        let ranges = Ranges::<Version>::try_from(
            &VersionSpec::from_str("<=1.2.3", ParseStrictness::Strict).unwrap(),
        )
        .unwrap();
        assert!(ranges.contains(&Version::from_str("1.0").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.2.3").unwrap()));
        assert!(!ranges.contains(&Version::from_str("2.0").unwrap()));
    }

    #[test]
    fn test_try_from_group_and() {
        let spec = VersionSpec::from_str(">=1.2.3,<2.0.0", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();

        assert!(!ranges.contains(&Version::from_str("1.2.0").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.2.3").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.9.0").unwrap()));
        assert!(!ranges.contains(&Version::from_str("2.0.0").unwrap()));
    }

    #[test]
    fn test_try_from_group_or() {
        let spec = VersionSpec::from_str(">=2.0.0|<1.0.0", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();

        assert!(ranges.contains(&Version::from_str("0.5").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1.5").unwrap()));
        assert!(ranges.contains(&Version::from_str("2.0.0").unwrap()));
        assert!(ranges.contains(&Version::from_str("3.0").unwrap()));
    }

    #[test]
    fn test_try_from_starts_with() {
        // 1.2.* => >=1.2.0a0,<1.3.0a0
        let spec = VersionSpec::from_str("1.2.*", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert!(ranges.contains(&Version::from_str("1.2.0").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.2.3").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.2.99").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1.3.0").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1.1.0").unwrap()));
    }

    #[test]
    fn test_try_from_not_starts_with() {
        let spec = VersionSpec::StrictRange(
            StrictRangeOperator::NotStartsWith,
            StrictVersion(Version::from_str("1.2").unwrap()),
        );
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert!(!ranges.contains(&Version::from_str("1.2.0").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1.2.3").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.3.0").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.1.0").unwrap()));
    }

    #[test]
    fn test_try_from_compatible() {
        // ~=2.4 => >=2.4.0a0,<3.0a0
        let spec = VersionSpec::from_str("~=2.4", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert!(ranges.contains(&Version::from_str("2.4").unwrap()));
        assert!(ranges.contains(&Version::from_str("2.5").unwrap()));
        assert!(ranges.contains(&Version::from_str("2.99").unwrap()));
        assert!(!ranges.contains(&Version::from_str("3.0").unwrap()));
        assert!(!ranges.contains(&Version::from_str("2.3").unwrap()));
    }

    #[test]
    fn test_try_from_compatible_single_segment() {
        let spec = VersionSpec::from_str("~=1", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        let expected = Ranges::<Version>::try_from(
            &VersionSpec::from_str(">=1,<1!0", ParseStrictness::Strict).unwrap(),
        )
        .unwrap();

        assert_eq!(ranges, expected);
        assert!(ranges.contains(&Version::from_str("1").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.2").unwrap()));
        assert!(ranges.contains(&Version::from_str("100").unwrap()));
        assert!(!ranges.contains(&Version::from_str("0.9").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1!0").unwrap()));
    }

    #[test]
    fn test_try_from_not_compatible() {
        let spec = VersionSpec::StrictRange(
            StrictRangeOperator::NotCompatible,
            StrictVersion(Version::from_str("2.4").unwrap()),
        );
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert!(!ranges.contains(&Version::from_str("2.4").unwrap()));
        assert!(!ranges.contains(&Version::from_str("2.5").unwrap()));
        assert!(ranges.contains(&Version::from_str("3.0").unwrap()));
        assert!(ranges.contains(&Version::from_str("2.3").unwrap()));
    }

    #[test]
    fn test_try_from_group_with_strict_range() {
        let spec = VersionSpec::Group(
            LogicalOperator::And,
            vec![
                VersionSpec::from_str(">=1.2.3", ParseStrictness::Strict).unwrap(),
                VersionSpec::from_str("1.2.*", ParseStrictness::Strict).unwrap(),
            ],
        );
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert!(ranges.contains(&Version::from_str("1.2.3").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.2.99").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1.2.2").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1.3.0").unwrap()));
    }

    #[test]
    fn test_try_from_matches_consistency_range() {
        let spec = VersionSpec::from_str(">=3.8,<4.0", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        for v_str in ["0.9", "1.0.0", "3.0", "3.8", "3.12", "3.99", "4.0"] {
            assert_eq!(
                spec.matches(&Version::from_str(v_str).unwrap()),
                ranges.contains(&Version::from_str(v_str).unwrap()),
                "mismatch for version={v_str}"
            );
        }
    }

    #[test]
    fn test_try_from_matches_consistency_exact() {
        let spec = VersionSpec::from_str("==1.0.0", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert_eq!(
            spec.matches(&Version::from_str("1.0.0").unwrap()),
            ranges.contains(&Version::from_str("1.0.0").unwrap())
        );
        assert_eq!(
            spec.matches(&Version::from_str("1.0.1").unwrap()),
            ranges.contains(&Version::from_str("1.0.1").unwrap())
        );
        assert_eq!(
            spec.matches(&Version::from_str("0.9").unwrap()),
            ranges.contains(&Version::from_str("0.9").unwrap())
        );
    }

    #[test]
    fn test_try_from_matches_consistency_not_equals() {
        let spec = VersionSpec::from_str("!=2.0.0", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert_eq!(
            spec.matches(&Version::from_str("2.0.0").unwrap()),
            ranges.contains(&Version::from_str("2.0.0").unwrap())
        );
        assert_eq!(
            spec.matches(&Version::from_str("1.0.0").unwrap()),
            ranges.contains(&Version::from_str("1.0.0").unwrap())
        );
        assert_eq!(
            spec.matches(&Version::from_str("3.0").unwrap()),
            ranges.contains(&Version::from_str("3.0").unwrap())
        );
    }

    #[test]
    fn test_try_from_matches_consistency_greater_than() {
        let spec = VersionSpec::from_str(">1.0", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert_eq!(
            spec.matches(&Version::from_str("0.9").unwrap()),
            ranges.contains(&Version::from_str("0.9").unwrap())
        );
        assert_eq!(
            spec.matches(&Version::from_str("1.0.0").unwrap()),
            ranges.contains(&Version::from_str("1.0.0").unwrap())
        );
        assert_eq!(
            spec.matches(&Version::from_str("2.0.0").unwrap()),
            ranges.contains(&Version::from_str("2.0.0").unwrap())
        );
    }

    #[test]
    fn test_try_from_matches_consistency_less_than() {
        let spec = VersionSpec::from_str("<3.0", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert_eq!(
            spec.matches(&Version::from_str("2.0.0").unwrap()),
            ranges.contains(&Version::from_str("2.0.0").unwrap())
        );
        assert_eq!(
            spec.matches(&Version::from_str("3.0").unwrap()),
            ranges.contains(&Version::from_str("3.0").unwrap())
        );
        assert_eq!(
            spec.matches(&Version::from_str("4.0").unwrap()),
            ranges.contains(&Version::from_str("4.0").unwrap())
        );
    }

    #[test]
    fn test_try_from_matches_consistency_or_group() {
        let spec = VersionSpec::from_str(">=1.0,<2.0|>=3.0,<4.0", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        for v_str in ["0.9", "1.0.0", "1.5", "2.0.0", "3.0", "3.99", "4.0"] {
            assert_eq!(
                spec.matches(&Version::from_str(v_str).unwrap()),
                ranges.contains(&Version::from_str(v_str).unwrap()),
                "mismatch for version={v_str}"
            );
        }
    }

    #[test]
    fn test_ranges_set_algebra() {
        let range_a = Ranges::<Version>::try_from(
            &VersionSpec::from_str(">=3.8,<4.0", ParseStrictness::Strict).unwrap(),
        )
        .unwrap();
        let range_b = Ranges::<Version>::try_from(
            &VersionSpec::from_str(">=3.12", ParseStrictness::Strict).unwrap(),
        )
        .unwrap();

        let combined = range_a.intersection(&range_b);

        assert!(!combined.contains(&Version::from_str("3.8").unwrap()));
        assert!(combined.contains(&Version::from_str("3.12").unwrap()));
        assert!(combined.contains(&Version::from_str("3.13").unwrap()));
        assert!(!combined.contains(&Version::from_str("4.0").unwrap()));

        assert!(!range_b.subset_of(&range_a));
        assert!(combined.subset_of(&range_a));
        assert!(combined.subset_of(&range_b));
    }

    #[test]
    fn test_try_from_nested_groups() {
        // (>=1,<2 | >=3,<4) AND >=1.5
        let spec = VersionSpec::Group(
            LogicalOperator::And,
            vec![
                VersionSpec::Group(
                    LogicalOperator::Or,
                    vec![
                        VersionSpec::from_str(">=1,<2", ParseStrictness::Strict).unwrap(),
                        VersionSpec::from_str(">=3,<4", ParseStrictness::Strict).unwrap(),
                    ],
                ),
                VersionSpec::from_str(">=1.5", ParseStrictness::Strict).unwrap(),
            ],
        );

        let ranges = Ranges::<Version>::try_from(&spec).unwrap();

        assert!(!ranges.contains(&Version::from_str("1.4").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.5").unwrap()));
        assert!(!ranges.contains(&Version::from_str("2.5").unwrap()));
        assert!(ranges.contains(&Version::from_str("3.5").unwrap()));
        assert!(!ranges.contains(&Version::from_str("4.0").unwrap()));
    }

    #[test]
    fn test_try_from_empty_group_and() {
        let spec = VersionSpec::Group(LogicalOperator::And, vec![]);
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert_eq!(ranges, Ranges::full());
    }

    #[test]
    fn test_try_from_empty_group_or() {
        let spec = VersionSpec::Group(LogicalOperator::Or, vec![]);
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_try_from_deeply_nested_strict_range() {
        // (>=1.0 | (<2.0 AND 1.2.*)) => full | [1.2.0a0, 1.3.0a0) ∩ (-∞, 2.0) => full
        // Actually >=1.0 already covers most, so union with anything is >=1.0
        let spec = VersionSpec::Group(
            LogicalOperator::And,
            vec![VersionSpec::Group(
                LogicalOperator::Or,
                vec![
                    VersionSpec::from_str(">=1.0", ParseStrictness::Strict).unwrap(),
                    VersionSpec::Group(
                        LogicalOperator::And,
                        vec![
                            VersionSpec::from_str("<2.0", ParseStrictness::Strict).unwrap(),
                            VersionSpec::from_str("1.2.*", ParseStrictness::Strict).unwrap(),
                        ],
                    ),
                ],
            )],
        );

        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert!(ranges.contains(&Version::from_str("1.0").unwrap()));
        assert!(ranges.contains(&Version::from_str("1.2.5").unwrap()));
        assert!(ranges.contains(&Version::from_str("2.0").unwrap()));
        assert!(!ranges.contains(&Version::from_str("0.9").unwrap()));
    }

    #[test]
    fn test_try_from_epoch() {
        let spec = VersionSpec::from_str(">=1!1.2,<1!2", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert!(ranges.contains(&Version::from_str("1!1.2.3").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1.2.3").unwrap()));
        assert!(!ranges.contains(&Version::from_str("1!2.0").unwrap()));
    }

    #[test]
    fn test_try_from_subset_of() {
        let outer = Ranges::<Version>::try_from(
            &VersionSpec::from_str(">=1.2,<3.0", ParseStrictness::Strict).unwrap(),
        )
        .unwrap();
        let inner = Ranges::<Version>::try_from(
            &VersionSpec::from_str(">=1.3,<2", ParseStrictness::Strict).unwrap(),
        )
        .unwrap();

        assert!(inner.subset_of(&outer));
        assert!(!outer.subset_of(&inner));

        assert!(inner.contains(&Version::from_str("1.5").unwrap()));
        assert!(!inner.contains(&Version::from_str("1.2").unwrap()));
        assert!(outer.contains(&Version::from_str("1.2").unwrap()));
        assert!(!outer.contains(&Version::from_str("3.0").unwrap()));
    }

    #[test]
    fn test_try_from_matches_consistency_starts_with() {
        let spec = VersionSpec::from_str("1.2.*", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        for v_str in [
            "1.1", "1.2", "1.2a0", "1.2.0", "1.2.0a0", "1.2.1", "1.2.99", "1.3", "1.3.0a0",
        ] {
            assert_eq!(
                spec.matches(&Version::from_str(v_str).unwrap()),
                ranges.contains(&Version::from_str(v_str).unwrap()),
                "mismatch for version={v_str}"
            );
        }
    }

    #[rstest]
    #[case("1.2.*", "1.2dev", true)]
    #[case("1.2.*", "1.2dev1", true)]
    #[case("1.2.*", "1.3dev", false)]
    #[case("=1.2a", "1.2a", true)]
    #[case("=1.2a", "1.2a1", true)]
    #[case("=1.2a", "1.2aa", false)]
    #[case("=1.2a", "1.2adev", false)]
    #[case("=1.2f", "1.2f1", true)]
    #[case("=1.2f", "1.2fa", false)]
    #[case("=1.2f", "1.2ff", false)]
    #[case("=1.2", "1.2dev", true)]
    #[case("=1.2", "1.2dev1", true)]
    #[case("=1.2", "1.3dev", false)]
    #[case("=1!1.2", "1!1.2dev", true)]
    #[case("=1!1.2", "1!1.3dev", false)]
    #[case("~=1.2a.3", "1.2a4", true)]
    #[case("~=1.2a.3", "1.2aa", false)]
    #[case("~=1.1", "1.2dev", true)]
    #[case("~=1.1", "2dev", false)]
    #[case("~=1.1", "2dev1", false)]
    fn test_try_from_matches_consistency_dev_boundaries(
        #[case] spec_str: &str,
        #[case] v_str: &str,
        #[case] expected: bool,
    ) {
        let spec = VersionSpec::from_str(spec_str, ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        let version = Version::from_str(v_str).unwrap();

        assert_eq!(
            spec.matches(&version),
            expected,
            "spec.matches mismatch for spec={spec_str}, version={v_str}"
        );
        assert_eq!(
            ranges.contains(&version),
            expected,
            "ranges.contains mismatch for spec={spec_str}, version={v_str}"
        );
    }

    #[rstest]
    #[case("=1.2dev", "1.2devdev")]
    #[case("=1.2a", "1.2a.dev")]
    #[ignore = "known limitation: non-numeric prefix lowering is still best-effort"]
    fn test_try_from_matches_consistency_non_numeric_prefix_boundaries(
        #[case] spec_str: &str,
        #[case] v_str: &str,
    ) {
        let spec = VersionSpec::from_str(spec_str, ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        assert_eq!(
            spec.matches(&Version::from_str(v_str).unwrap()),
            ranges.contains(&Version::from_str(v_str).unwrap()),
            "mismatch for spec={spec_str}, version={v_str}"
        );
    }

    #[test]
    fn test_try_from_matches_consistency_compatible() {
        let spec = VersionSpec::from_str("~=1.1", ParseStrictness::Strict).unwrap();
        let ranges = Ranges::<Version>::try_from(&spec).unwrap();
        for v_str in [
            "1.0", "1.1a0", "1.1", "1.1.0", "1.1.1", "1.1.2", "1.2", "1.2.0a0", "2.0",
        ] {
            assert_eq!(
                spec.matches(&Version::from_str(v_str).unwrap()),
                ranges.contains(&Version::from_str(v_str).unwrap()),
                "mismatch for version={v_str}"
            );
        }
    }

    #[test]
    fn test_from_ranges_round_trip_non_strict_semantics() {
        let candidate_versions = [
            "0.9", "1.0", "1.0.0", "1.2.3", "1.5", "1.9.9", "2.0", "2.4", "3.0", "1!1.0",
        ];
        let specs = vec![
            VersionSpec::None,
            VersionSpec::Any,
            VersionSpec::from_str("==1.2.3", ParseStrictness::Strict).unwrap(),
            VersionSpec::from_str("!=1.2.3", ParseStrictness::Strict).unwrap(),
            VersionSpec::from_str(">1.2.3", ParseStrictness::Strict).unwrap(),
            VersionSpec::from_str(">=1.2.3", ParseStrictness::Strict).unwrap(),
            VersionSpec::from_str("<2.0.0", ParseStrictness::Strict).unwrap(),
            VersionSpec::from_str("<=2.0.0", ParseStrictness::Strict).unwrap(),
            VersionSpec::from_str(">=1.2.3,<2.0.0", ParseStrictness::Strict).unwrap(),
            VersionSpec::from_str(">=2.0.0|<1.0.0", ParseStrictness::Strict).unwrap(),
            VersionSpec::from_str(">=1,<2|>=3,<4", ParseStrictness::Strict).unwrap(),
        ];

        for spec in specs {
            let round_trip = VersionSpec::from(Ranges::<Version>::try_from(&spec).unwrap());
            assert_same_matches(&spec, &round_trip, &candidate_versions);
        }
    }

    #[test]
    fn test_from_ranges_edge_cases() {
        let singleton = Version::from_str("1.2.3").unwrap();

        assert_eq!(
            VersionSpec::from(Ranges::<Version>::empty()),
            VersionSpec::None
        );
        assert_eq!(
            VersionSpec::from(Ranges::<Version>::full()),
            VersionSpec::Any
        );
        assert_eq!(
            VersionSpec::from(Ranges::<Version>::singleton(singleton.clone())),
            VersionSpec::Exact(EqualityOperator::Equals, singleton)
        );
    }

    #[test]
    fn test_from_ranges_not_equals_multi_segment() {
        let value = Version::from_str("1.0").unwrap();
        let spec = VersionSpec::from_str("!=1.0", ParseStrictness::Strict).unwrap();

        let round_trip = VersionSpec::from(Ranges::<Version>::try_from(&spec).unwrap());

        assert_eq!(
            round_trip,
            VersionSpec::Group(
                LogicalOperator::Or,
                vec![
                    VersionSpec::Range(RangeOperator::Less, value.clone()),
                    VersionSpec::Range(RangeOperator::Greater, value),
                ],
            )
        );
    }

    #[test]
    fn test_from_ranges_lossy_round_trip_strict_range_semantics() {
        let candidate_versions = [
            "1.1", "1.2", "1.2a0", "1.2.0", "1.2.9", "1.3", "2.3", "2.4", "2.5", "3.0",
        ];

        for source in ["1.2.*", "~=2.4"] {
            let strict_spec = VersionSpec::from_str(source, ParseStrictness::Strict).unwrap();
            let round_trip = VersionSpec::from(Ranges::<Version>::try_from(&strict_spec).unwrap());

            assert_matches!(
                &round_trip,
                VersionSpec::Group(LogicalOperator::And, parts)
                    if matches!(parts.as_slice(), [
                        VersionSpec::Range(RangeOperator::GreaterEquals, _),
                        VersionSpec::Range(RangeOperator::Less, _)
                    ])
            );
            assert_same_matches(&strict_spec, &round_trip, &candidate_versions);
        }
    }

    #[test]
    fn test_from_ranges_adjacent_or_simplifies_to_single_range() {
        let original = VersionSpec::from_str(">=1,<2|>=2", ParseStrictness::Strict).unwrap();
        let round_trip = VersionSpec::from(Ranges::<Version>::try_from(&original).unwrap());
        let expected = VersionSpec::from_str(">=1", ParseStrictness::Strict).unwrap();

        assert_eq!(round_trip, expected);
        assert_same_matches(
            &original,
            &round_trip,
            &["0.9", "1", "1.5", "1.999", "2", "3.0", "1!1"],
        );
    }

    mod proptest_fuzz {
        use std::fmt::Display;
        use std::str::FromStr;

        use proptest::prelude::*;
        use version_ranges::Ranges;

        use crate::{
            version::StrictVersion,
            version_spec::{EqualityOperator, LogicalOperator, RangeOperator, StrictRangeOperator},
            Version, VersionSpec,
        };

        /// Wrapper that uses `Display` for `Debug` output, so proptest
        /// failure messages show the human-readable representation.
        #[derive(Clone)]
        struct Show<T: Display>(T);

        impl<T: Display> std::fmt::Debug for Show<T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        /// Strategy that generates a `Version` by building a version string and
        /// parsing it. This avoids needing to construct the complex internal
        /// representation directly while guaranteeing every generated value is
        /// a valid `Version`.
        fn arb_version() -> impl Strategy<Value = Show<Version>> {
            let epoch = prop_oneof![
                9 => Just(String::new()),
                1 => (1..=2u64).prop_map(|e| format!("{e}!")),
            ];

            let segment_count = 1..=4usize;

            let segments = segment_count.prop_flat_map(|n| {
                proptest::collection::vec(0..=20u64, n)
                    .prop_map(|nums| {
                        nums.iter()
                            .map(|n| n.to_string())
                            .collect::<Vec<_>>()
                            .join(".")
                    })
            });

            let suffix = prop_oneof![
                6 => Just(String::new()),
                1 => Just("dev".to_string()),
                1 => Just("post".to_string()),
                1 => prop_oneof![
                    Just("a"), Just("rc"), Just("f"), Just("alpha"),
                ].prop_map(|s| s.to_string()),
                1 => (
                    prop_oneof![Just("a"), Just("rc"), Just("dev"), Just("post")],
                    0..=3u64,
                ).prop_map(|(s, n)| format!("{s}{n}")),
            ];

            let local = prop_oneof![
                8 => Just(String::new()),
                1 => Just("+local".to_string()),
                1 => Just("+1".to_string()),
            ];

            (epoch, segments, suffix, local).prop_map(|(e, segs, suf, loc)| {
                let s = format!("{e}{segs}{suf}{loc}");
                Show(
                    Version::from_str(&s)
                        .unwrap_or_else(|_| panic!("generated invalid version string: {s}")),
                )
            })
        }

        /// Strategy that generates a `VersionSpec` AST, using `prop_recursive`
        /// to keep depth and branching small.
        fn arb_version_spec() -> impl Strategy<Value = Show<VersionSpec>> {
            let leaf = prop_oneof![
                1 => Just(VersionSpec::None),
                1 => Just(VersionSpec::Any),
                4 => (
                    prop_oneof![
                        Just(EqualityOperator::Equals),
                        Just(EqualityOperator::NotEquals),
                    ],
                    arb_version(),
                ).prop_map(|(op, v)| VersionSpec::Exact(op, v.0)),
                4 => (
                    prop_oneof![
                        Just(RangeOperator::Greater),
                        Just(RangeOperator::GreaterEquals),
                        Just(RangeOperator::Less),
                        Just(RangeOperator::LessEquals),
                    ],
                    arb_version(),
                ).prop_map(|(op, v)| VersionSpec::Range(op, v.0)),
                4 => (
                    prop_oneof![
                        Just(StrictRangeOperator::StartsWith),
                        Just(StrictRangeOperator::NotStartsWith),
                        Just(StrictRangeOperator::Compatible),
                        Just(StrictRangeOperator::NotCompatible),
                    ],
                    arb_version(),
                ).prop_map(|(op, v)| VersionSpec::StrictRange(op, StrictVersion(v.0))),
            ];

            leaf.prop_recursive(
                3,  // max depth
                16, // max nodes
                4,  // items per collection
                |inner| {
                    (
                        prop_oneof![
                            Just(LogicalOperator::And),
                            Just(LogicalOperator::Or),
                        ],
                        proptest::collection::vec(inner, 0..=4),
                    )
                        .prop_map(|(op, children)| VersionSpec::Group(op, children))
                },
            )
            .prop_map(Show)
        }

        /// Derive boundary-biased "witness" versions from a spec's endpoints.
        fn boundary_versions(spec: &VersionSpec) -> Vec<Version> {
            let mut versions = Vec::new();
            collect_versions_from_spec(spec, &mut versions);
            let mut extras = Vec::new();
            for v in &versions {
                // dev-appended
                if let Ok(bumped) = v.bump(crate::VersionBumpType::Last) {
                    extras.push(bumped);
                }
                // version with extra trailing .0
                if let Ok(extended) = v.extend_to_length(v.segment_count() + 1) {
                    extras.push(extended.into_owned());
                }
            }
            versions.extend(extras);
            versions.sort();
            versions.dedup();
            versions
        }

        fn collect_versions_from_spec(spec: &VersionSpec, out: &mut Vec<Version>) {
            match spec {
                VersionSpec::None | VersionSpec::Any => {}
                VersionSpec::Exact(_, v) | VersionSpec::Range(_, v) => {
                    out.push(v.clone());
                }
                VersionSpec::StrictRange(_, sv) => {
                    out.push(sv.0.clone());
                }
                VersionSpec::Group(_, children) => {
                    for child in children {
                        collect_versions_from_spec(child, out);
                    }
                }
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(500))]

            /// For every generated VersionSpec and set of Versions, assert
            /// `spec.matches(v) == ranges.contains(v)` after converting spec
            /// to `Ranges<Version>`. Skip if `TryFrom` returns `Err`.
            #[test]
            fn matches_agrees_with_ranges(
                spec in arb_version_spec(),
                random_versions in proptest::collection::vec(arb_version(), 1..=10),
            ) {
                let spec = &spec.0;
                let ranges = match Ranges::<Version>::try_from(spec) {
                    Ok(r) => r,
                    Err(_) => return Ok(()),
                };

                let mut candidates: Vec<Version> =
                    random_versions.into_iter().map(|v| v.0).collect();
                candidates.extend(boundary_versions(spec));

                for v in &candidates {
                    let spec_match = spec.matches(v);
                    let range_match = ranges.contains(v);
                    prop_assert_eq!(
                        spec_match,
                        range_match,
                        "mismatch for version={}",
                        v,
                    );
                }
            }

            /// `VersionSpec → Ranges<Version> → VersionSpec` preserves match
            /// behavior for all candidate versions.
            #[test]
            fn spec_to_ranges_round_trip_preserves_semantics(
                spec in arb_version_spec(),
                random_versions in proptest::collection::vec(arb_version(), 1..=10),
            ) {
                let spec = &spec.0;
                let ranges = match Ranges::<Version>::try_from(spec) {
                    Ok(r) => r,
                    Err(_) => return Ok(()),
                };
                let round_tripped = VersionSpec::from(ranges);

                let mut candidates: Vec<Version> =
                    random_versions.into_iter().map(|v| v.0).collect();
                candidates.extend(boundary_versions(spec));

                for v in &candidates {
                    prop_assert_eq!(
                        spec.matches(v),
                        round_tripped.matches(v),
                        "round-trip mismatch for version={}, round_tripped={}",
                        v, round_tripped,
                    );
                }
            }

            /// Build `Ranges<Version>` from a spec, convert back to
            /// `VersionSpec`, then to `Ranges` again and assert exact equality.
            #[test]
            fn ranges_to_spec_to_ranges_is_exact(
                spec in arb_version_spec(),
            ) {
                let spec = &spec.0;
                let ranges = match Ranges::<Version>::try_from(spec) {
                    Ok(r) => r,
                    Err(_) => return Ok(()),
                };
                let spec2 = VersionSpec::from(ranges.clone());
                let ranges2 = Ranges::<Version>::try_from(&spec2)
                    .expect("round-tripped spec should always convert back to Ranges");
                prop_assert_eq!(
                    ranges,
                    ranges2,
                    "Ranges round-trip not exact, intermediate spec={}",
                    spec2,
                );
            }
        }
    }
}
