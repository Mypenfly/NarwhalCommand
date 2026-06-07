//!@Open: ./tests/data/rust_complex.rs

// 场景 1：Location:Block @75 行号指向 get_connection 方法，Block 自动扩展
// 在其后插入 is_healthy 方法
//!@Location:Block @75,75
//!@New:
    pub fn is_healthy(&self) -> bool {
        self.active > 0 && self.active <= self.config.max_connections as usize
    }
//!@Off:Location

// 场景 2：内容匹配 + Location:Block 定位 impl AppConfig 中 validate 方法块，
// Delete:Block 删除该方法块，再 New 替换
//!@Location:
impl AppConfig {
//!@Location:Block
    pub fn validate(&self) -> Result<(), String> {
//!@Delete:Block
//!@New:
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("name is empty".into());
        }
        if self.max_connections == 0 {
            return Err("max_connections is zero".into());
        }
        Ok(())
    }

    pub fn set_default_name(&mut self, name: &str) {
        self.name = name.to_string();
    }
//!@Off:Location
//!@Off:Location

// 场景 3：Location:Block 定位 TransformStage impl 块，
// 嵌套行号 Delete + New 在 process 方法中添加 Capitalize 分支
//!@Location:Block
impl ProcessStage for TransformStage {
//!@Location:Block
    fn process(&self, input: &str) -> Result<String, String> {
//!@Delete:@3,7
//!@New:
            let result = match self.transform_type {
                TransformType::Uppercase => input.to_uppercase(),
                TransformType::Lowercase => input.to_lowercase(),
                TransformType::Trim => input.trim().to_string(),
                TransformType::Reverse => input.chars().rev().collect(),
                TransformType::Capitalize => {
                    let mut chars: Vec<char> = input.chars().collect();
                    if let Some(first) = chars.first_mut() {
                        *first = first.to_uppercase().next().unwrap_or(*first);
                    }
                    chars.into_iter().collect()
                }
            };
//!@Off:Location
//!@Off:Location

//!@Off:Open
