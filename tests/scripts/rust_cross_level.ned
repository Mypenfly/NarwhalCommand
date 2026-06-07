//!@Open: ./tests/data/rust_complex.rs

// 场景 1：Location:Block 定位 impl DataPipeline，然后 New:End 追加两个新方法
//!@Location:Block
impl DataPipeline {
//!@New:End
    pub fn clear_stages(&mut self) {
        self.stages.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
//!@Off:Location

// 场景 2：三级嵌套进入 execute 方法内的 match 分支，替换错误处理
//!@Location:
impl DataPipeline {
//!@Location:
    pub fn execute(&self, input: &str) -> Result<String, String> {
//!@Location:
                Err(e) => {
                    return Err(format!("stage '{}' failed: {}", stage.name(), e));
                }
//!@Delete:
                Err(e) => {
                    return Err(format!("stage '{}' failed: {}", stage.name(), e));
                }
//!@New:
                Err(e) => {
                    log::error!("stage '{}' error: {}", stage.name(), e);
                    return Err(format!("pipeline aborted at '{}': {}", stage.name(), e));
                }
//!@Off:Location
//!@Off:Location
//!@Off:Location

//!@Off:Open
