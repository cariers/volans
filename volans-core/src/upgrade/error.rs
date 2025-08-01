use std::fmt;

use volans_stream_select::NegotiationError;

#[derive(Debug)]
pub enum UpgradeError<E> {
    Select(NegotiationError),
    Apply(E),
}

impl<E> UpgradeError<E> {
    pub fn map_err<F, T>(self, f: F) -> UpgradeError<T>
    where
        F: FnOnce(E) -> T,
    {
        match self {
            UpgradeError::Select(e) => UpgradeError::Select(e),
            UpgradeError::Apply(e) => UpgradeError::Apply(f(e)),
        }
    }

    pub fn into_err<T>(self) -> UpgradeError<T>
    where
        T: From<E>,
    {
        self.map_err(Into::into)
    }
}

impl<E> fmt::Display for UpgradeError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpgradeError::Select(_) => write!(f, "Stream select failed"),
            UpgradeError::Apply(_) => write!(f, "Handshake failed"),
        }
    }
}

impl<E> std::error::Error for UpgradeError<E>
where
    E: std::error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            UpgradeError::Select(e) => Some(e),
            UpgradeError::Apply(e) => Some(e),
        }
    }
}

impl<E> From<NegotiationError> for UpgradeError<E> {
    fn from(e: NegotiationError) -> Self {
        UpgradeError::Select(e)
    }
}
