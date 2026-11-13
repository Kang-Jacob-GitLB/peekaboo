// 파일 정보·폴더 목록. Electron 의 src/main/file-service.ts 와 같은 결과(JSON 모양 포함)를 낸다.

use std::fs;
use std::io;
use std::path::Path;
use std::time::SystemTime;

use serde::Serialize;

/// `PeekabooApi` 의 `FileInfo`
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub path: String,
    pub name: String,
    /// 소문자, 점 제외("png"). 확장자 없으면 ""
    pub ext: String,
    pub size: u64,
    pub is_directory: bool,
    pub mtime_ms: f64,
}

/// `PeekabooApi` 의 `DirEntry`
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub is_directory: bool,
    pub size: u64,
    pub mtime_ms: f64,
}

/// `PeekabooApi.listDir` 의 결과
#[derive(Debug, Serialize, PartialEq)]
pub struct DirListing {
    pub entries: Vec<DirEntry>,
    pub truncated: bool,
}

fn mtime_ms(meta: &fs::Metadata) -> f64 {
    match meta
        .modified()
        .map(|t| t.duration_since(SystemTime::UNIX_EPOCH))
    {
        Ok(Ok(d)) => d.as_secs_f64() * 1000.0,
        Ok(Err(before)) => -before.duration().as_secs_f64() * 1000.0,
        Err(_) => 0.0,
    }
}

/// "a.PNG" → "png", ".gitignore" → "gitignore", "Makefile" → "" (Electron 쪽 fileExt 와 같은 규칙)
fn file_ext(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_lowercase(),
        Some(_) => name[1..].to_lowercase(),
        None => String::new(),
    }
}

/// 심볼릭 링크는 따라간다(Node 의 stat 과 같다).
pub fn file_info(path: &str) -> io::Result<FileInfo> {
    let meta = fs::metadata(path)?;
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| path.to_owned());
    let is_directory = meta.is_dir();
    Ok(FileInfo {
        path: path.to_owned(),
        ext: if is_directory {
            String::new()
        } else {
            file_ext(&name)
        },
        name,
        size: meta.len(),
        is_directory,
        mtime_ms: mtime_ms(&meta),
    })
}

/// 앞에서부터 최대 `max` 개. 개별 항목의 stat 이 실패하면(끊어진 링크 등) 크기·시각 0 으로 채운다.
pub fn list_dir(path: &str, max: u32) -> io::Result<DirListing> {
    let mut iter = fs::read_dir(path)?;
    let mut entries = Vec::new();
    for dirent in iter.by_ref().take(max as usize) {
        let dirent = dirent?;
        let name = dirent.file_name().to_string_lossy().into_owned();
        entries.push(match fs::metadata(dirent.path()) {
            Ok(m) => DirEntry {
                name,
                is_directory: m.is_dir(),
                size: m.len(),
                mtime_ms: mtime_ms(&m),
            },
            Err(_) => DirEntry {
                name,
                is_directory: dirent.file_type().map(|t| t.is_dir()).unwrap_or(false),
                size: 0,
                mtime_ms: 0.0,
            },
        });
    }
    let truncated = iter.next().is_some();
    Ok(DirListing { entries, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("peekaboo-files-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 확장자_규칙() {
        assert_eq!(file_ext("a.PNG"), "png");
        assert_eq!(file_ext(".gitignore"), "gitignore");
        assert_eq!(file_ext("Makefile"), "");
        assert_eq!(file_ext("archive.tar.GZ"), "gz");
        assert_eq!(file_ext("보고서.최종.TXT"), "txt");
    }

    #[test]
    fn 파일_정보는_camel_case_json_이다() {
        let dir = scratch("info");
        let file = dir.join("메모 #1.MD");
        fs::write(&file, b"hello").unwrap();
        let info = file_info(file.to_str().unwrap()).unwrap();
        assert_eq!(
            (
                info.name.as_str(),
                info.ext.as_str(),
                info.size,
                info.is_directory
            ),
            ("메모 #1.MD", "md", 5, false)
        );
        assert!(info.mtime_ms > 0.0);
        let json = serde_json::to_value(&info).unwrap();
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for k in ["path", "name", "ext", "size", "isDirectory", "mtimeMs"] {
            assert!(keys.contains(&k), "{k}");
        }
        let dir_info = file_info(dir.to_str().unwrap()).unwrap();
        assert!(dir_info.is_directory);
        assert_eq!(dir_info.ext, "");
        assert!(file_info(dir.join("없음").to_str().unwrap()).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 폴더_목록은_max_개까지만_주고_넘으면_truncated() {
        let dir = scratch("list");
        for i in 0..5 {
            fs::write(dir.join(format!("f{i}.txt")), vec![0u8; i]).unwrap();
        }
        fs::create_dir(dir.join("sub")).unwrap();
        let all = list_dir(dir.to_str().unwrap(), 100).unwrap();
        assert_eq!((all.entries.len(), all.truncated), (6, false));
        assert!(all
            .entries
            .iter()
            .any(|e| e.name == "sub" && e.is_directory));
        let part = list_dir(dir.to_str().unwrap(), 4).unwrap();
        assert_eq!((part.entries.len(), part.truncated), (4, true));
        let exact = list_dir(dir.to_str().unwrap(), 6).unwrap();
        assert_eq!((exact.entries.len(), exact.truncated), (6, false));
        assert!(list_dir(dir.join("없음").to_str().unwrap(), 4).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
