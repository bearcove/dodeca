//! Stem - compact string type for DOM content.

use compact_str::CompactString;
use facet::Facet;
use std::borrow::Borrow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use tendril::StrTendril;

/// Compact string type used for text content, comments, and attribute values.
///
/// Can be either borrowed (zero-copy from input) or owned (after mutation).
#[derive(Clone, Facet)]
#[facet(cow)]
#[repr(u8)]
pub enum Stem<'a> {
    Borrowed(&'a str),
    Owned(CompactString),
}

impl<'a> Stem<'a> {
    pub fn new() -> Self {
        Self::Owned(CompactString::default())
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(s) => s,
            Self::Owned(s) => s.as_str(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    pub fn len(&self) -> usize {
        self.as_str().len()
    }

    pub fn push_str(&mut self, s: &str) {
        match self {
            Self::Owned(existing) => {
                existing.push_str(s);
            }
            Self::Borrowed(borrowed) => {
                *self = Self::Owned(compact_str::format_compact!("{}{}", borrowed, s));
            }
        }
    }

    pub fn push_tendril(&mut self, t: &StrTendril) {
        self.push_str(t.as_ref());
    }

    /// Convert to an owned version with 'static lifetime.
    pub fn into_owned(self) -> Stem<'static> {
        match self {
            Self::Borrowed(s) => Stem::Owned(CompactString::new(s)),
            Self::Owned(s) => Stem::Owned(s),
        }
    }
}

impl Default for Stem<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for Stem<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for Stem<'_> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for Stem<'_> {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for Stem<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Stem<'_> {}

impl PartialEq<str> for Stem<'_> {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Stem<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl Hash for Stem<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl fmt::Debug for Stem<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Stem<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl<'a> From<&'a str> for Stem<'a> {
    fn from(s: &'a str) -> Self {
        Self::Borrowed(s)
    }
}

impl From<String> for Stem<'_> {
    fn from(s: String) -> Self {
        Self::Owned(CompactString::from(s))
    }
}

impl From<CompactString> for Stem<'_> {
    fn from(s: CompactString) -> Self {
        Self::Owned(s)
    }
}

impl From<StrTendril> for Stem<'_> {
    fn from(t: StrTendril) -> Self {
        Self::Owned(CompactString::new(t.as_ref()))
    }
}

impl From<&StrTendril> for Stem<'_> {
    fn from(t: &StrTendril) -> Self {
        Self::Owned(CompactString::new(t.as_ref()))
    }
}

const _: () = {
    const fn assert_send<T: Send>() {}
    const fn assert_sync<T: Sync>() {}
    assert_send::<Stem<'static>>();
    assert_sync::<Stem<'static>>();
};
