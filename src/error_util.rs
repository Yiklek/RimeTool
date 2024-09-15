pub(crate) trait AnyhowExt<T> {
    fn anyhow(self) -> Result<T, anyhow::Error>;
}

impl<T, E> AnyhowExt<T> for Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn anyhow(self) -> Result<T, anyhow::Error> {
        self.map_err(anyhow::Error::from)
    }
}
