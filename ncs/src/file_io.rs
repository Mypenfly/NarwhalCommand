//! 文件读写工具函数 (File I/O)
//!
//! 提供文件读写的辅助工具函数，统一处理错误类型。
//!
//! ## 实现逻辑
//!
//! 1. `read_file` — 安全读取文件全部内容
//! 2. `write_file` — 安全写入文件全部内容
//! 3. `path_exists` — 检查路径是否存在
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §9（模块架构）

use crate::error::FileError;

/// 安全读取文件全部内容
pub fn read_file(path: &str) -> Result<String, FileError> {
    std::fs::read_to_string(path).map_err(|e| FileError::CannotOpen {
        path: path.to_string(),
        reason: e.to_string(),
    })
}

/// 安全写入文件全部内容
pub fn write_file(path: &str, content: &str) -> Result<(), FileError> {
    std::fs::write(path, content).map_err(|e| FileError::WriteFailed {
        path: path.to_string(),
        reason: e.to_string(),
    })
}

/// 检查路径是否存在
pub fn path_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_file_success() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "line1").unwrap();
        writeln!(tmp, "line2").unwrap();
        let path = tmp.path().to_str().unwrap();

        let content = read_file(path).unwrap();
        assert_eq!(content, "line1\nline2\n");
    }

    #[test]
    fn test_read_file_not_found() {
        let result = read_file("/nonexistent/path/file.txt");
        assert!(result.is_err());
        let err = result.unwrap_err();
        let title = err.title();
        assert!(
            title.contains("无法打开文件"),
            "Expected CannotOpen, got: {}",
            title
        );
    }

    #[test]
    fn test_write_file_success() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();

        write_file(path, "hello world").unwrap();
        let content = std::fs::read_to_string(path).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_write_file_overwrite() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "old content").unwrap();
        let path = tmp.path().to_str().unwrap();

        write_file(path, "new content").unwrap();
        let content = std::fs::read_to_string(path).unwrap();
        assert_eq!(content, "new content");
    }

    #[test]
    fn test_path_exists_true() {
        let tmp = NamedTempFile::new().unwrap();
        assert!(path_exists(tmp.path().to_str().unwrap()));
    }

    #[test]
    fn test_path_exists_false() {
        assert!(!path_exists("/nonexistent/path/xyz.abc"));
    }
}
