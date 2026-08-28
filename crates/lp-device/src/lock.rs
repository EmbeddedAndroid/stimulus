use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

pub struct FileLock {
    file: File,
}

impl FileLock {
    pub fn acquire(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        file.try_lock_exclusive()?;
        Ok(Self { file })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excludes_second_owner_and_releases_on_drop() -> io::Result<()> {
        let path = std::env::temp_dir().join(format!("lp-device-lock-{}", std::process::id()));
        let first = FileLock::acquire(&path)?;
        assert!(FileLock::acquire(&path).is_err());
        drop(first);
        let second = FileLock::acquire(&path)?;
        drop(second);
        std::fs::remove_file(path)
    }
}
