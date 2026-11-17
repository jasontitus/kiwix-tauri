// SPDX-License-Identifier: GPL-3.0-or-later
//! Native desktop picker preferences. Mobile document URIs never enter this module.

use crate::store::Store;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
enum DialogKind {
    Import,
    Save,
}

impl DialogKind {
    fn parse(kind: &str) -> Result<Self, String> {
        match kind {
            "import" => Ok(Self::Import),
            "save" => Ok(Self::Save),
            _ => Err("Unknown desktop dialog kind".into()),
        }
    }
}

fn native_path(path: &str) -> Result<&Path, String> {
    if path.is_empty() || path.len() > 32_768 || path.contains('\0') {
        return Err("Invalid desktop dialog path".into());
    }
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err("Desktop dialog paths must be native absolute paths".into());
    }
    Ok(path)
}

fn selected_parent(path: &str) -> Result<String, String> {
    let path = native_path(path)?;
    if path.file_name().is_none() {
        return Err("Desktop dialog selection must name a file".into());
    }
    path.parent()
        .and_then(Path::to_str)
        .filter(|parent| !parent.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Desktop dialog parent is not a native Unicode path".into())
}

pub(crate) fn default_path(
    store: &Store,
    kind: &str,
    suggested_name: Option<&str>,
) -> Result<String, String> {
    let kind = DialogKind::parse(kind)?;
    let settings = store.settings()?;
    // A private picker must not expose the normal profile's last visited folder.
    let remembered = if settings.private_mode {
        None
    } else {
        match kind {
            DialogKind::Import => settings.last_import_directory.as_deref(),
            DialogKind::Save => settings.last_save_directory.as_deref(),
        }
        .filter(|path| native_path(path).is_ok_and(Path::is_dir))
    };
    let directory = remembered.map(PathBuf::from).or_else(|| {
        let dirs = directories::UserDirs::new()?;
        match kind {
            DialogKind::Import => dirs.download_dir().filter(|path| path.is_dir()).map(Path::to_path_buf),
            DialogKind::Save => dirs.document_dir().filter(|path| path.is_dir()).map(Path::to_path_buf),
        }
    });
    let mut path = match directory {
        Some(directory) => directory,
        None => std::env::current_dir().map_err(|error| error.to_string())?,
    };
    if let Some(name) = suggested_name {
        if name.is_empty() || name.len() > 4096 || name.contains('\0') {
            return Err("Invalid suggested export filename".into());
        }
        // Suggested filenames cannot override the remembered directory.
        let name = Path::new(name)
            .file_name()
            .ok_or("Invalid suggested export filename")?;
        path.push(name);
    }
    let path = path.to_str().ok_or("Desktop dialog path is not Unicode")?;
    native_path(path)?;
    Ok(path.to_owned())
}

pub(crate) fn remember_directory(store: &Store, kind: &str, file: &str) -> Result<(), String> {
    let kind = DialogKind::parse(kind)?;
    let settings = store.settings()?;
    // Selecting/exporting a file is distinct from permission to persist preferences.
    if settings.private_mode || settings.read_only {
        return Ok(());
    }
    let directory = selected_parent(file)?;
    let previous = match kind {
        DialogKind::Import => &settings.last_import_directory,
        DialogKind::Save => &settings.last_save_directory,
    };
    if previous.as_deref() == Some(directory.as_str()) {
        return Ok(());
    }
    // No portable destination validation: user exports may be outside app data.
    store.update(|state| {
        match kind {
            DialogKind::Import => state.settings.last_import_directory = Some(directory),
            DialogKind::Save => state.settings.last_save_directory = Some(directory),
        }
        Ok(())
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn profile() -> PathBuf {
        std::env::temp_dir().join(format!("kiwix-desktop-workflow-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn remembers_independent_picker_parents_across_restart() {
        let root = profile();
        let store = Store::new(root.clone()).unwrap();
        let import = root.join("imports/book.zim");
        let export = root.join("exports/article.html");
        fs::create_dir(import.parent().unwrap()).unwrap();
        fs::create_dir(export.parent().unwrap()).unwrap();
        remember_directory(&store, "import", import.to_str().unwrap()).unwrap();
        remember_directory(&store, "save", export.to_str().unwrap()).unwrap();
        drop(store);
        let store = Store::new(root.clone()).unwrap();
        assert_eq!(default_path(&store, "import", None).unwrap(), root.join("imports").to_str().unwrap());
        assert_eq!(default_path(&store, "save", Some("next.html")).unwrap(), root.join("exports").join("next.html").to_str().unwrap());
        // Remembering the choice does not create or claim to save an export.
        assert!(!export.exists());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unavailable_remembered_directory_falls_back_without_rewriting_preferences() {
        let root = profile();
        let store = Store::new(root.clone()).unwrap();
        let missing = root.join("removed");
        fs::create_dir(&missing).unwrap();
        for kind in ["save", "import"] {
            let fallback = default_path(&store, kind, Some("article.pdf")).unwrap();
            remember_directory(&store, kind, missing.join("previous.pdf").to_str().unwrap()).unwrap();
            assert_eq!(default_path(&store, kind, None).unwrap(), missing.to_str().unwrap());
            fs::remove_dir(&missing).unwrap();
            let before = fs::read(root.join("state.json")).unwrap();
            assert_eq!(default_path(&store, kind, Some("article.pdf")).unwrap(), fallback);
            assert_eq!(fs::read(root.join("state.json")).unwrap(), before);
            // A mounted path replaced by a regular file is unavailable too.
            fs::write(&missing, b"not a directory").unwrap();
            assert_eq!(default_path(&store, kind, Some("article.pdf")).unwrap(), fallback);
            fs::remove_file(&missing).unwrap();
            fs::create_dir(&missing).unwrap();
        }
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_and_read_only_do_not_persist_picker_choices() {
        let root = profile();
        let store = Store::new(root.clone()).unwrap();
        remember_directory(&store, "import", root.join("normal/book.zim").to_str().unwrap()).unwrap();
        store.update(|state| { state.settings.private_mode = true; Ok(()) }).unwrap();
        let before = fs::read(root.join("state.json")).unwrap();
        remember_directory(&store, "import", root.join("secret/book.zim").to_str().unwrap()).unwrap();
        assert_eq!(before, fs::read(root.join("state.json")).unwrap());
        assert_ne!(default_path(&store, "import", None).unwrap(), root.join("normal").to_str().unwrap());
        store.update(|state| { state.settings.private_mode = false; Ok(()) }).unwrap();
        let before = fs::read(root.join("state.json")).unwrap();
        drop(store);
        let store = Store::new_with_mode(root.clone(), true).unwrap();
        remember_directory(&store, "save", root.join("readonly/export.json").to_str().unwrap()).unwrap();
        assert_eq!(before, fs::read(root.join("state.json")).unwrap());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_uris_and_relative_selections() {
        assert!(selected_parent("content://provider/document/book.zim").is_err());
        assert!(selected_parent("book.zim").is_err());
        assert!(selected_parent("/bad\0/book.zim").is_err());
    }
}
