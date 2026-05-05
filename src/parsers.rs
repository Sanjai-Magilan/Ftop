use crate::MemInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    MissingField(&'static str),
    InvalidNumber(String),
    InvalidFormat(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::MissingField(field) => write!(f, "missing field: {}", field),
            ParseError::InvalidNumber(value) => write!(f, "invalid number: {}", value),
            ParseError::InvalidFormat(msg) => write!(f, "invalid format: {}", msg),
        }
    }
}

impl std::error::Error for ParseError {}

#[cfg_attr(not(test), allow(dead_code))]
pub fn parse_proc_stat_totals(content: &str) -> Result<(u64, u64), ParseError> {
    let cpu_line = content
        .lines()
        .find(|line| line.starts_with("cpu "))
        .ok_or(ParseError::MissingField("cpu"))?;

    let mut fields = cpu_line.split_whitespace();
    let _cpu = fields.next().ok_or(ParseError::MissingField("cpu label"))?;

    let nums = fields
        .map(|value| value.parse::<u64>().map_err(|_| ParseError::InvalidNumber(value.to_string())))
        .collect::<Result<Vec<_>, _>>()?;

    if nums.len() < 4 {
        return Err(ParseError::InvalidFormat(
            "cpu line must include at least four numeric fields".to_string(),
        ));
    }

    let idle = nums[3].saturating_add(*nums.get(4).unwrap_or(&0));
    let total = nums.iter().copied().fold(0_u64, u64::saturating_add);
    Ok((total, idle))
}

pub fn parse_proc_stat_tail_fields(content: &str) -> Result<Vec<&str>, ParseError> {
    let end_comm = content
        .rfind(") ")
        .ok_or(ParseError::InvalidFormat("missing command terminator".to_string()))?;
    let tail = &content[end_comm + 2..];
    let fields = tail.split_whitespace().collect::<Vec<_>>();
    if fields.is_empty() {
        return Err(ParseError::InvalidFormat(
            "proc stat tail has no fields".to_string(),
        ));
    }
    Ok(fields)
}

pub fn parse_parent_pid_from_stat(content: &str) -> Result<i32, ParseError> {
    let fields = parse_proc_stat_tail_fields(content)?;
    if fields.len() < 2 {
        return Err(ParseError::MissingField("ppid"));
    }
    fields[1]
        .parse::<i32>()
        .map_err(|_| ParseError::InvalidNumber(fields[1].to_string()))
}

pub fn parse_process_status(content: &str) -> Result<bool, ParseError> {
    for line in content.lines() {
        if line.starts_with("State:") {
            let state = line
                .split_whitespace()
                .nth(1)
                .ok_or(ParseError::MissingField("state"))?;
            return Ok(state == "T");
        }
    }

    Err(ParseError::MissingField("State"))
}

pub fn parse_priority_from_stat(content: &str) -> Result<i64, ParseError> {
    let fields = parse_proc_stat_tail_fields(content)?;
    if fields.len() < 16 {
        return Err(ParseError::MissingField("priority"));
    }
    fields[15]
        .parse::<i64>()
        .map_err(|_| ParseError::InvalidNumber(fields[15].to_string()))
}

pub fn parse_statm_shared_pages(content: &str) -> Result<u64, ParseError> {
    let parts = content.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 3 {
        return Err(ParseError::MissingField("statm shared pages"));
    }
    parts[2]
        .parse::<u64>()
        .map_err(|_| ParseError::InvalidNumber(parts[2].to_string()))
}

pub fn parse_meminfo(content: &str) -> Result<MemInfo, ParseError> {
    let mut total = 0_u64;
    let mut free = 0_u64;
    let mut buffers = 0_u64;
    let mut cached = 0_u64;
    let mut sreclaimable = 0_u64;
    let mut shmem = 0_u64;
    let mut available = 0_u64;

    for line in content.lines() {
        let mut parts = line.split(':');
        let Some(key) = parts.next().map(|v| v.trim()) else {
            continue;
        };
        let Some(val_part) = parts.next().map(|v| v.trim()) else {
            continue;
        };
        let Some(kb_str) = val_part.split_whitespace().next() else {
            continue;
        };
        let Ok(kb) = kb_str.parse::<u64>() else {
            continue;
        };
        let bytes = kb.saturating_mul(1024);
        match key {
            "MemTotal" => total = bytes,
            "MemFree" => free = bytes,
            "Buffers" => buffers = bytes,
            "Cached" => cached = bytes,
            "SReclaimable" => sreclaimable = bytes,
            "Shmem" => shmem = bytes,
            "MemAvailable" => available = bytes,
            _ => {}
        }
    }

    if total == 0 {
        return Err(ParseError::MissingField("MemTotal"));
    }

    Ok(MemInfo {
        total,
        free,
        buffers,
        cached,
        sreclaimable,
        shmem,
        available,
    })
}
