#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not initialized; run `engram init`")]
    NotInitialized,
    #[error("index busy")]
    IndexBusy,
    #[error("{0}")]
    Usage(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("db: {0}")]
    Db(String),
}

impl Error {
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Usage(_) => 1,
            Error::NotInitialized => 2,
            Error::IndexBusy | Error::Io(_) | Error::Db(_) => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_spec() {
        assert_eq!(Error::Usage("x".into()).exit_code(), 1);
        assert_eq!(Error::NotInitialized.exit_code(), 2);
        assert_eq!(Error::IndexBusy.exit_code(), 3);
        assert_eq!(Error::Db("locked".into()).exit_code(), 3);
    }
}
