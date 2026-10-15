use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextMatchMode {
    Exact,
    Contains,
    #[default]
    Lcs,
}

/// 目标文字由调用方先完成翻译；不去除空白，也不把文字当作正则。
pub fn matches_text(
    target: &str,
    recognized: &str,
    mode: TextMatchMode,
    ignore_case: bool,
    lcs_percent: f64,
) -> bool {
    if target.is_empty() || recognized.is_empty() {
        return false;
    }
    let target_len = target.chars().count();
    let (target, recognized) = if ignore_case {
        (target.to_lowercase(), recognized.to_lowercase())
    } else {
        (target.to_owned(), recognized.to_owned())
    };
    match mode {
        TextMatchMode::Exact => recognized == target,
        TextMatchMode::Contains => recognized.contains(&target),
        TextMatchMode::Lcs => {
            if !lcs_percent.is_finite() || !(0.0..=1.0).contains(&lcs_percent) {
                return false;
            }
            let recognized: Vec<_> = recognized.chars().collect();
            let mut lengths = vec![0usize; recognized.len() + 1];
            for a in target.chars() {
                let mut diagonal = 0;
                for (index, b) in recognized.iter().enumerate() {
                    let previous = lengths[index + 1];
                    lengths[index + 1] = if a == *b {
                        diagonal + 1
                    } else {
                        lengths[index + 1].max(lengths[index])
                    };
                    diagonal = previous;
                }
            }
            // 与旧 find_by_lcs 一致，分母为大小写转换前目标的 Unicode 字符数。
            lengths[recognized.len()] as f64 >= target_len as f64 * lcs_percent
        }
    }
}
