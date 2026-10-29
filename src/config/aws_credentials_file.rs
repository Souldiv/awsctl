use anyhow::{Context, Result};
use ini::Ini;
use std::path::{Path, PathBuf};

const DEFAULT_SECTION: &str = "default";

pub fn aws_credentials_path() -> Result<PathBuf> {
    let home_dir = directories::UserDirs::new()
        .context("Unable to get home directory")?
        .home_dir()
        .join(".aws");

    Ok(home_dir.join("credentials"))
}

pub fn write_default_credentials(
    credentials_path: &Path,
    access_key_id: &str,
    secret_access_key: &str,
    session_token: &str,
) -> Result<()> {
    let mut credentials = load_or_create(credentials_path)?;

    credentials.delete(Some(DEFAULT_SECTION));
    credentials
        .with_section(Some(DEFAULT_SECTION))
        .set("aws_access_key_id", access_key_id)
        .set("aws_secret_access_key", secret_access_key)
        .set("aws_session_token", session_token);

    if let Some(parent) = credentials_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| "Unable to create the .aws directory.")?;
    }

    credentials
        .write_to_file(credentials_path)
        .with_context(|| "Unable to write the AWS credentials file.")?;

    Ok(())
}

fn load_or_create(credentials_path: &Path) -> Result<Ini> {
    if !credentials_path.try_exists()? {
        return Ok(Ini::new());
    }

    Ini::load_from_file(credentials_path).with_context(|| {
        format!(
            "Unable to read {}. Please fix or move it and try again.",
            credentials_path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::write_default_credentials;
    use ini::Ini;
    use std::path::{Path, PathBuf};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "awsctl-credentials-{}-{}-{:?}",
                label,
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp dir");
            TempDir { path }
        }

        fn file(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    const FIXTURE: &str = "[default]\n\
        aws_access_key_id = AKIAOLD\n\
        aws_secret_access_key = oldsecret\n\
        aws_session_token = oldtoken\n\
        region = ap-southeast-2\n\n\
        [hand-written]\n\
        aws_access_key_id = AKIAHAND\n\
        aws_secret_access_key = handsecret\n\n\
        [au]\n\
        aws_access_key_id = AKIAAU\n\
        aws_secret_access_key = ausecret\n";

    fn value(credentials_path: &Path, section: &str, key: &str) -> Option<String> {
        Ini::load_from_file(credentials_path)
            .expect("credentials parse")
            .section(Some(section))
            .and_then(|properties| properties.get(key))
            .map(String::from)
    }

    #[test]
    fn replaces_default_and_leaves_the_other_sections_alone() {
        let dir = TempDir::new("replaces");
        let credentials_path = dir.file("credentials");
        std::fs::write(&credentials_path, FIXTURE).expect("seed credentials");

        write_default_credentials(&credentials_path, "AKIANEW", "newsecret", "newtoken")
            .expect("writes");

        assert_eq!(
            value(&credentials_path, "default", "aws_access_key_id").as_deref(),
            Some("AKIANEW")
        );
        assert_eq!(
            value(&credentials_path, "default", "aws_secret_access_key").as_deref(),
            Some("newsecret")
        );
        assert_eq!(
            value(&credentials_path, "default", "aws_session_token").as_deref(),
            Some("newtoken")
        );
        assert_eq!(value(&credentials_path, "default", "region"), None);

        assert_eq!(
            value(&credentials_path, "hand-written", "aws_access_key_id").as_deref(),
            Some("AKIAHAND")
        );
        assert_eq!(
            value(&credentials_path, "hand-written", "aws_secret_access_key").as_deref(),
            Some("handsecret")
        );
        assert_eq!(
            value(&credentials_path, "au", "aws_access_key_id").as_deref(),
            Some("AKIAAU")
        );

        let credentials = Ini::load_from_file(&credentials_path).expect("credentials parse");
        assert_eq!(credentials.section_all(Some("default")).count(), 1);
        assert_eq!(credentials.sections().flatten().count(), 3);
    }

    #[test]
    fn creates_the_file_when_it_is_missing() {
        let dir = TempDir::new("creates");
        let credentials_path = dir.file("nested/credentials");

        write_default_credentials(&credentials_path, "AKIANEW", "newsecret", "newtoken")
            .expect("writes");

        assert_eq!(
            value(&credentials_path, "default", "aws_access_key_id").as_deref(),
            Some("AKIANEW")
        );
    }
}
