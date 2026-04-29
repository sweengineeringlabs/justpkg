use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageSpec {
    pub name: String,
    pub constraint: VersionConstraint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionConstraint {
    Any,
    Eq(String),
    Gte(String),
    Gt(String),
    Lte(String),
    Lt(String),
    Tilde(String),
}

impl std::fmt::Display for VersionConstraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Any        => write!(f, "*"),
            Self::Eq(v)      => write!(f, "={v}"),
            Self::Gte(v)     => write!(f, ">={v}"),
            Self::Gt(v)      => write!(f, ">{v}"),
            Self::Lte(v)     => write!(f, "<={v}"),
            Self::Lt(v)      => write!(f, "<{v}"),
            Self::Tilde(v)   => write!(f, "~{v}"),
        }
    }
}
