//! Write 命令
//!
//! 将内容写入指定文件。
//!
//! ## 实现逻辑
//!
//! 1. Normal 模式：将块内容写入指定路径，自动创建父目录
//! 2. Raw 模式：从下一行到 EOF 的全部内容原样写入
//! 3. 值输出，结果不保留
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.9 "Write", INSTRUCTION.md §3

use crate::error::NcsError;
use crate::parser::WriteMode;

/// Write 命令的执行入口
pub fn execute(_mode: WriteMode, path: &str, content: Option<&str>) -> Result<(), NcsError> {
    let content = content.unwrap_or("");
    write_content(path, content)
}

/// 将内容写入文件，自动创建父目录
fn write_content(path: &str, content: &str) -> Result<(), NcsError> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            NcsError::File(crate::error::FileError::WriteFailed {
                path: path.to_string(),
                reason: format!("无法创建父目录: {}", e),
            })
        })?;
    }
    crate::file_io::write_file(path, content)?;
    Ok(())
}
