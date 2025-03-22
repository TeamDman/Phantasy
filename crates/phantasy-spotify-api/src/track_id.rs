use std::ops::Deref;

#[derive(Debug)]
pub struct TrackId(pub String);
impl std::fmt::Display for TrackId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl Deref for TrackId {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl AsRef<str> for TrackId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}