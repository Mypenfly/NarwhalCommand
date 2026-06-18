// complex_indent.rs — 复杂缩进测试数据
//
// 包含多层嵌套 if/match/loop、不规整缩进、混合缩进级别，
// 用于验证 NCS 的 diff_taps 计算和缩进保持能力。

pub struct DeepNesting;

impl DeepNesting {
    pub fn process_levels(&self, input: i32) -> Result<String, String> {
        if input < 0 {
            return Err("negative input".into());
        }

        let mut result = String::new();

        if input == 0 {
            result.push_str("zero");
        } else if input < 10 {
            result.push_str("small: ");
            if input % 2 == 0 {
                result.push_str("even");
                if input % 4 == 0 {
                    result.push_str("_divisible_by_4");
                    if input % 8 == 0 {
                        result.push_str("_divisible_by_8");
                    } else {
    result.push_str("_not_divisible_by_8");
                    }
                }
            } else {
                result.push_str("odd");
            }
        } else if input < 100 {
            result.push_str("medium");
            match input % 3 {
                0 => {
                    if input % 5 == 0 {
                        result.push_str("_fizzbuzz");
                    } else {
                        result.push_str("_fizz");
                    }
                }
                1 => result.push_str("_mod1"),
                _ => {
                    if input % 5 == 0 {
                        result.push_str("_buzz");
                    } else {
                        result.push_str("_other");
                    }
                }
            }
        } else {
            result.push_str("large");
            for i in 0..3 {
                if i == 0 {
  result.push_str("_first");
                } else if i == 1 {
                    result.push_str("_second");
                } else {
                    result.push_str("_third");
                }
            }
        }

        Ok(result)
    }

    pub fn check_consistency(&self, a: i32, b: i32) -> Vec<String> {
        let mut results = Vec::new();

        match a.cmp(&b) {
            std::cmp::Ordering::Less => {
                results.push("less".into());
                for i in 0..a {
                    if i % 2 == 0 {
              results.push(format!("even_{}", i));
                    } else {
                        results.push(format!("odd_{}", i));
                    }
                }
            }
            std::cmp::Ordering::Equal => {
                results.push("equal".into());
            }
            std::cmp::Ordering::Greater => {
                results.push("greater".into());
                let mut count = 0;
                while count < b {
                    match count % 3 {
                        0 => results.push(format!("mod3_0_{}", count)),
                        1 => {
                            if count > 5 {
                                results.push(format!("mod3_1_big_{}", count));
                            } else {
    results.push(format!("mod3_1_small_{}", count));
                            }
                        }
                        _ => results.push(format!("mod3_2_{}", count)),
                    }
                    count += 1;
                }
            }
        }

        results
    }
}
