//! Read only the selected airport's source records; retain normal scenery precedence.
use std::io::{BufRead, BufReader, Read};

pub(super) fn read(path: &str, airport: &str) -> Result<String, String> {
    if std::path::Path::new(path)
        .file_name()
        .is_none_or(|name| name != "apt.dat")
    {
        return std::fs::read_to_string(path).map_err(|e| e.to_string());
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 2 * 1024 * 1024 * 1024 {
        return Err(format!("Airport source exceeds 2 GiB: {path}"));
    }
    records(BufReader::with_capacity(256 * 1024, file), airport).map_err(|e| format!("{path}: {e}"))
}

fn records(mut source: impl BufRead, airport: &str) -> Result<String, String> {
    let mut line = String::new();
    let mut selected = false;
    let mut result = String::new();
    loop {
        line.clear();
        if source
            .by_ref()
            .take(65_537)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?
            == 0
        {
            break;
        }
        if line.len() > 65_536 {
            return Err("Airport record exceeds 64 KiB".into());
        }
        // Most of apt.dat is pavement geometry. Avoid tokenising those millions of
        // lines while finding a header, then let the unchanged core parser validate it.
        let text = line.trim_start();
        let header = text.strip_prefix('1').is_some_and(|rest| {
            rest.starts_with(char::is_whitespace)
                || rest
                    .strip_prefix(['6', '7'])
                    .is_some_and(|rest| rest.starts_with(char::is_whitespace))
        });
        if header {
            if selected {
                break;
            }
            selected = text.split_whitespace().nth(4) == Some(airport);
        }
        if selected {
            if result.len() + line.len() > 32 * 1024 * 1024 {
                return Err("Airport records exceed 32 MiB".into());
            }
            result.push_str(&line);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_exact_airport_without_changing_header_or_geometry() {
        let wanted = " 1 12 0 0 WANT Example\r\n1302 datum_lat 1.25\r\n110 1 0 0 Pavement\r\n111 1.2 3.4\r\n";
        let text =
            format!("I\n1200\n1 1 0 0 OTHER Elsewhere\n111 8 9\n{wanted}16 0 0 0 SEAP Sea\n99\n");
        assert_eq!(records(text.as_bytes(), "WANT").unwrap(), wanted);
        assert_eq!(records(text.as_bytes(), "MISSING").unwrap(), "");
        assert!(records(text.as_bytes(), "SEAP").unwrap().starts_with("16 "));
        let source = format!("17 0 0 0 HELI Pad\n{}", "x".repeat(65_537));
        assert!(records(source.as_bytes(), "HELI").is_err());
    }
}
