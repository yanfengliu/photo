use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub name: String,
    pub photos: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct CollectionStore {
    pub collections: Vec<Collection>,
}

impl CollectionStore {
    pub fn create(&mut self, name: &str) {
        self.collections.push(Collection {
            name: name.to_string(),
            photos: Vec::new(),
        });
        self.sort();
    }

    pub fn rename(&mut self, index: usize, new_name: &str) {
        if let Some(c) = self.collections.get_mut(index) {
            c.name = new_name.to_string();
        }
        self.sort();
    }

    pub fn delete(&mut self, index: usize) {
        if index < self.collections.len() {
            self.collections.remove(index);
        }
    }

    pub fn add_photo(&mut self, collection_index: usize, path: &Path) {
        if let Some(c) = self.collections.get_mut(collection_index) {
            let pb = path.to_path_buf();
            if !c.photos.contains(&pb) {
                c.photos.push(pb);
            }
        }
    }

    pub fn remove_photo(&mut self, collection_index: usize, path: &Path) {
        if let Some(c) = self.collections.get_mut(collection_index) {
            c.photos.retain(|p| p != path);
        }
    }

    pub fn next_default_name(&self) -> String {
        let base = "New Collection";
        if !self.collections.iter().any(|c| c.name == base) {
            return base.to_string();
        }
        for i in 2.. {
            let name = format!("{base} {i}");
            if !self.collections.iter().any(|c| c.name == name) {
                return name;
            }
        }
        unreachable!()
    }

    pub fn save_to(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.collections) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn load_from(path: &Path) -> Self {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        let collections: Vec<Collection> = serde_json::from_str(&content).unwrap_or_default();
        CollectionStore { collections }
    }

    pub fn save(&self) {
        if let Some(path) = collections_file_path() {
            self.save_to(&path);
        }
    }

    pub fn load() -> Self {
        match collections_file_path() {
            Some(path) => Self::load_from(&path),
            None => Self::default(),
        }
    }

    fn sort(&mut self) {
        self.collections
            .sort_by(|a, b| natord::compare(&a.name, &b.name));
    }
}

pub fn collections_file_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|dir| Path::new(&dir).join("photo").join("collections.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_adds_collection_sorted() {
        let mut store = CollectionStore::default();
        store.create("Zebra");
        store.create("Alpha");
        assert_eq!(store.collections.len(), 2);
        assert_eq!(store.collections[0].name, "Alpha");
        assert_eq!(store.collections[1].name, "Zebra");
    }

    #[test]
    fn create_empty_photos() {
        let mut store = CollectionStore::default();
        store.create("Test");
        assert!(store.collections[0].photos.is_empty());
    }

    #[test]
    fn rename_resorts() {
        let mut store = CollectionStore::default();
        store.create("Alpha");
        store.create("Beta");
        store.rename(0, "Zeta");
        assert_eq!(store.collections[0].name, "Beta");
        assert_eq!(store.collections[1].name, "Zeta");
    }

    #[test]
    fn rename_out_of_bounds_no_panic() {
        let mut store = CollectionStore::default();
        store.rename(99, "Nope");
        assert!(store.collections.is_empty());
    }

    #[test]
    fn delete_removes_collection() {
        let mut store = CollectionStore::default();
        store.create("A");
        store.create("B");
        store.create("C");
        store.delete(1);
        assert_eq!(store.collections.len(), 2);
        assert_eq!(store.collections[0].name, "A");
        assert_eq!(store.collections[1].name, "C");
    }

    #[test]
    fn delete_out_of_bounds_no_panic() {
        let mut store = CollectionStore::default();
        store.delete(0);
        assert!(store.collections.is_empty());
    }

    #[test]
    fn add_photo_no_duplicates() {
        let mut store = CollectionStore::default();
        store.create("Test");
        let path = PathBuf::from("/photo/a.jpg");
        store.add_photo(0, &path);
        store.add_photo(0, &path);
        assert_eq!(store.collections[0].photos.len(), 1);
    }

    #[test]
    fn add_photo_out_of_bounds_no_panic() {
        let mut store = CollectionStore::default();
        store.add_photo(99, &PathBuf::from("/a.jpg"));
    }

    #[test]
    fn remove_photo_keeps_others() {
        let mut store = CollectionStore::default();
        store.create("Test");
        let a = PathBuf::from("/photo/a.jpg");
        let b = PathBuf::from("/photo/b.jpg");
        store.add_photo(0, &a);
        store.add_photo(0, &b);
        store.remove_photo(0, &a);
        assert_eq!(store.collections[0].photos, vec![b]);
    }

    #[test]
    fn remove_photo_not_present_no_panic() {
        let mut store = CollectionStore::default();
        store.create("Test");
        store.remove_photo(0, &PathBuf::from("/not_here.jpg"));
        assert!(store.collections[0].photos.is_empty());
    }

    #[test]
    fn next_default_name_increments() {
        let mut store = CollectionStore::default();
        assert_eq!(store.next_default_name(), "New Collection");
        store.create("New Collection");
        assert_eq!(store.next_default_name(), "New Collection 2");
        store.create("New Collection 2");
        assert_eq!(store.next_default_name(), "New Collection 3");
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("collections.json");

        let mut store = CollectionStore::default();
        store.create("Vacation");
        store.add_photo(0, &PathBuf::from("/photo/a.jpg"));
        store.add_photo(0, &PathBuf::from("/photo/b.png"));
        store.create("Work");
        store.add_photo(1, &PathBuf::from("/photo/c.jpg"));
        store.save_to(&file);

        let loaded = CollectionStore::load_from(&file);
        assert_eq!(loaded.collections.len(), 2);
        assert_eq!(loaded.collections[0].name, "Vacation");
        assert_eq!(loaded.collections[0].photos.len(), 2);
        assert_eq!(loaded.collections[1].name, "Work");
        assert_eq!(loaded.collections[1].photos.len(), 1);
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let store = CollectionStore::load_from(Path::new("/nonexistent/collections.json"));
        assert!(store.collections.is_empty());
    }

    #[test]
    fn load_corrupt_json_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("collections.json");
        std::fs::write(&file, "not valid json{{{").unwrap();
        let store = CollectionStore::load_from(&file);
        assert!(store.collections.is_empty());
    }
}
