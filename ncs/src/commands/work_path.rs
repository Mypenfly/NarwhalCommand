//! WorkPath 命令
//!
//! 修改进程当前工作目录，同时更新引擎的 work_path 基准。
//!
//! ## 实现逻辑
//!
//! 1. 验证路径存在（若为文件则取其父目录）
//! 2. 返回有效路径供引擎更新 work_path
//! 3. 引擎负责调用 set_current_dir 和更新 work_path
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.11 "WorkPath", INSTRUCTION.md §3

use crate::error::{FileError, NcsError};
use std::path::{Path, PathBuf};

/// WorkPath 命令的执行入口，返回应设置的工作路径
pub fn resolve(path: &str) -> Result<PathBuf, NcsError> {
    let p = Path::new(path);

    let target = if p.exists() {
        if p.is_dir() {
            p.to_path_buf()
        } else {
            p.parent()
                .map(|parent| parent.to_path_buf())
                .unwrap_or_else(|| p.to_path_buf())
        }
    } else {
        return Err(NcsError::File(FileError::NotFound {
            path: path.to_string(),
        }));
    };

    Ok(target)
}
