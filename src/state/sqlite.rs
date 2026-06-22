use std::path::Path;

use crate::Result;

pub(crate) fn ensure_sqlite_parent(raw_url: &str) -> Result<()> {
    let Some(path) = sqlite_file_path(raw_url) else {
        return Ok(());
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

pub(crate) fn sea_orm_sqlite_url(raw_url: &str) -> String {
    if raw_url == "sqlite::memory:" || raw_url.starts_with("sqlite://") {
        return raw_url.to_string();
    }
    if let Some(path) = raw_url.strip_prefix("sqlite:") {
        return format!("sqlite://{path}?mode=rwc");
    }
    raw_url.to_string()
}

fn sqlite_file_path(raw_url: &str) -> Option<&Path> {
    if raw_url == "sqlite::memory:" {
        return None;
    }
    raw_url
        .strip_prefix("sqlite:")
        .or_else(|| raw_url.strip_prefix("sqlite://"))
        .and_then(|path| path.split('?').next())
        .map(Path::new)
}

#[cfg(test)]
mod tests {
    use super::sea_orm_sqlite_url;

    #[test]
    fn converts_sqlite_url_for_sea_orm() {
        assert_eq!(
            sea_orm_sqlite_url("sqlite:data/node.db"),
            "sqlite://data/node.db?mode=rwc"
        );
    }
}
