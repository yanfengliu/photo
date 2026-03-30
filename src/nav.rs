use std::path::{Path, PathBuf};

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "svg", "svgz", "ico", "tga",
    "qoi", "hdr", "exr",
];

pub struct DirNav {
    files: Vec<PathBuf>,
    index: usize,
}

impl DirNav {
    pub fn new(path: &Path) -> Self {
        let dir = if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        };

        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| is_image_file(p))
            .collect();

        files.sort_by(|a, b| {
            natord::compare(
                a.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                b.file_name().and_then(|n| n.to_str()).unwrap_or(""),
            )
        });

        let index = files.iter().position(|p| p == path).unwrap_or(0);

        DirNav { files, index }
    }

    pub fn next(&mut self) -> Option<PathBuf> {
        if self.files.is_empty() {
            return None;
        }
        self.index = (self.index + 1) % self.files.len();
        Some(self.files[self.index].clone())
    }

    pub fn prev(&mut self) -> Option<PathBuf> {
        if self.files.is_empty() {
            return None;
        }
        self.index = if self.index == 0 {
            self.files.len() - 1
        } else {
            self.index - 1
        };
        Some(self.files[self.index].clone())
    }

    pub fn current_filename(&self) -> String {
        self.files
            .get(self.index)
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string()
    }

    pub fn current_index(&self) -> usize {
        self.index
    }

    pub fn count(&self) -> usize {
        self.files.len()
    }
}

fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_dir(names: &[&str]) -> (tempfile::TempDir, Vec<PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for name in names {
            let p = dir.path().join(name);
            std::fs::write(&p, b"").unwrap();
            paths.push(p);
        }
        (dir, paths)
    }

    #[test]
    fn scans_only_image_files() {
        let (dir, _) = setup_dir(&["photo.jpg", "notes.txt", "icon.png", "data.csv"]);

        let nav = DirNav::new(&dir.path().join("photo.jpg"));
        assert_eq!(nav.count(), 2); // jpg + png only
    }

    #[test]
    fn natural_sort_order() {
        let (dir, _) = setup_dir(&["img10.png", "img2.png", "img1.png"]);

        let nav = DirNav::new(&dir.path().join("img1.png"));
        // Natural sort: img1, img2, img10
        assert_eq!(nav.current_filename(), "img1.png");
        assert_eq!(nav.current_index(), 0);
    }

    #[test]
    fn next_cycles_forward() {
        let (dir, _) = setup_dir(&["a.png", "b.png", "c.png"]);

        let mut nav = DirNav::new(&dir.path().join("a.png"));
        assert_eq!(nav.current_filename(), "a.png");

        let next = nav.next().unwrap();
        assert!(next.ends_with("b.png"));
        assert_eq!(nav.current_filename(), "b.png");

        nav.next();
        assert_eq!(nav.current_filename(), "c.png");

        // Wraps around
        nav.next();
        assert_eq!(nav.current_filename(), "a.png");
    }

    #[test]
    fn prev_cycles_backward() {
        let (dir, _) = setup_dir(&["a.png", "b.png", "c.png"]);

        let mut nav = DirNav::new(&dir.path().join("a.png"));
        // Wraps to end
        nav.prev();
        assert_eq!(nav.current_filename(), "c.png");

        nav.prev();
        assert_eq!(nav.current_filename(), "b.png");
    }

    #[test]
    fn starts_at_given_file() {
        let (dir, _) = setup_dir(&["a.png", "b.png", "c.png"]);

        let nav = DirNav::new(&dir.path().join("b.png"));
        assert_eq!(nav.current_filename(), "b.png");
        assert_eq!(nav.current_index(), 1);
    }

    #[test]
    fn empty_directory_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut nav = DirNav::new(&dir.path().join("nonexistent.png"));

        assert_eq!(nav.count(), 0);
        assert!(nav.next().is_none());
        assert!(nav.prev().is_none());
        assert_eq!(nav.current_filename(), "");
    }

    #[test]
    fn case_insensitive_extensions() {
        let (dir, _) = setup_dir(&["photo.JPG", "image.Png"]);

        let nav = DirNav::new(&dir.path().join("photo.JPG"));
        assert_eq!(nav.count(), 2);
    }
}
