/// Parse duration string (e.g. "3-5分钟", "10分钟") into seconds range.
pub fn parse_duration(duration_str: &str) -> (u32, u32) {
    let s = duration_str.trim();

    if let Some(range) = parse_range(s) {
        return range;
    }

    if let Some(single) = parse_single(s) {
        return (single, single);
    }

    (180, 300)
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    let re = regex_lite::Regex::new(r"(\d+)\s*[-–—]\s*(\d+)\s*分钟?").ok()?;
    let caps = re.captures(s)?;
    let min = caps.get(1)?.as_str().parse::<u32>().ok()?;
    let max = caps.get(2)?.as_str().parse::<u32>().ok()?;
    Some((min * 60, max * 60))
}

fn parse_single(s: &str) -> Option<u32> {
    let re = regex_lite::Regex::new(r"(\d+)\s*分钟?").ok()?;
    let caps = re.captures(s)?;
    let val = caps.get(1)?.as_str().parse::<u32>().ok()?;
    Some(val * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_3to5_minutes() {
        let (min, max) = parse_duration("3-5分钟");
        assert_eq!(min, 180);
        assert_eq!(max, 300);
    }

    #[test]
    fn test_parse_10_minutes() {
        let (min, max) = parse_duration("10分钟");
        assert_eq!(min, 600);
        assert_eq!(max, 600);
    }

    #[test]
    fn test_parse_empty() {
        let (min, _max) = parse_duration("");
        assert_eq!(min, 180);
    }
}
