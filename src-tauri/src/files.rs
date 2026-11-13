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

/// Zone.Identifier 내용에서 ZoneId 를 읽어 3(인터넷) 이상인지.
/// 원본(Electron 의 isFromInternet: `/^\s*ZoneId\s*=\s*(\d+)/im`, 첫 일치)과 같거나 더 보수적이다.
/// - 줄 앞의 공백·BOM(`U+FEFF`)은 무시하고 키 대소문자는 가리지 않는다.
/// - 값은 앞쪽 숫자열만 본다(`3;`, `3 ` 도 3). 숫자가 아니거나 `ZoneId=` 줄이 없으면 false.
/// - 숫자가 u32 를 넘치면 아주 큰 영역 번호이므로 인터넷으로 본다(원본의 `Number(..) >= 3` 도 참).
/// - `ZoneId` 줄이 여러 개면 하나라도 3 이상이면 true(원본은 첫 줄만 본다. 더 보수적).
pub fn zone_is_internet(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{FEFF}');
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        if !key.trim().eq_ignore_ascii_case("ZoneId") {
            return false;
        }
        let value = value.trim_start();
        let end = value
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(value.len());
        let digits = &value[..end];
        if digits.is_empty() {
            return false;
        }
        digits.parse::<u32>().map_or(true, |zone| zone >= 3)
    })
}

/// 스트림 바이트를 문자열로. UTF-16LE BOM(FF FE)이면 UTF-16 으로, 아니면 UTF-8(깨진 바이트는 U+FFFD)로 읽는다.
/// 한 줄이 깨져도 다른 줄의 `ZoneId` 판정에 영향이 없게 하려고 `read_to_string` 대신 바이트로 읽는다.
fn decode_zone_stream(bytes: &[u8]) -> String {
    match bytes {
        [0xFF, 0xFE, rest @ ..] => {
            // 홀수 바이트 꼬리는 버린다
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| u16::from_le_bytes(*p))
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// NTFS 대체 데이터 스트림 `Zone.Identifier`(Mark of the Web)의 ZoneId 가 3(인터넷) 이상인지.
/// 스트림이 없거나 읽을 수 없으면(로컬 파일, FAT32 등) false. Electron 의 isFromInternet 과 같다.
/// 인터넷에서 받은 파일은 Office 등 외부 파서를 돌리지 않는다(탐색기도 같은 이유로 미리 보기 창을 막는다).
pub fn is_from_internet(path: &str) -> bool {
    fs::read(format!("{path}:Zone.Identifier"))
        .is_ok_and(|bytes| zone_is_internet(&decode_zone_stream(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn zone_id_3_이상만_인터넷() {
        assert!(zone_is_internet(
            "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=x\r\n"
        ));
        assert!(zone_is_internet("[ZoneTransfer]\nzoneid = 4\n"));
        assert!(!zone_is_internet("[ZoneTransfer]\r\nZoneId=2\r\n"));
        assert!(!zone_is_internet("[ZoneTransfer]\r\nZoneId=abc\r\n"));
        assert!(!zone_is_internet(
            "[ZoneTransfer]\r\nReferrerUrl=ZoneId=3\r\n"
        ));
        assert!(!zone_is_internet(""));
    }

    #[test]
    fn zone_id_는_원본_정규식보다_좁지_않게_읽는다() {
        // 뒤에 ; 나 공백이 붙어도 앞쪽 숫자열만 본다
        assert!(zone_is_internet("[ZoneTransfer]\r\nZoneId=3;\r\n"));
        assert!(zone_is_internet("ZoneId=3 \n"));
        assert!(zone_is_internet("  \t ZoneId \t=\t 4abc\n"));
        assert!(!zone_is_internet("ZoneId=2;\n"));
        assert!(!zone_is_internet("ZoneId=;3\n"));
        // 첫 줄의 BOM
        assert!(zone_is_internet("\u{FEFF}ZoneId=3\n"));
        assert!(zone_is_internet("\u{FEFF}[ZoneTransfer]\nZONEID=3\n"));
        // 넘치는 큰 수는 인터넷으로 본다
        assert!(zone_is_internet("ZoneId=99999999999999999999999\n"));
        assert!(zone_is_internet("ZoneId=4294967296\n"));
        // 음수는 앞쪽 숫자열이 아니다
        assert!(!zone_is_internet("ZoneId=-5\n"));
        // 여러 줄이면 하나라도 3 이상이면 true(더 보수적)
        assert!(zone_is_internet("ZoneId=2\nZoneId=3\n"));
    }

    fn utf16le_bom(s: &str) -> Vec<u8> {
        let mut v = vec![0xFF, 0xFE];
        v.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        v
    }

    #[test]
    fn 스트림_바이트_해석은_깨진_utf8_과_utf16_을_견딘다() {
        // 깨진 UTF-8 바이트가 섞여도 다른 줄의 ZoneId 를 읽는다
        let broken = b"\xC3\x28junk\r\nReferrerUrl=http://x/\xFF\xFE\x80\r\nZoneId=3\r\n";
        assert!(zone_is_internet(&decode_zone_stream(broken)));
        // UTF-16LE + BOM
        let three = utf16le_bom("[ZoneTransfer]\r\nZoneId=3\r\n");
        assert!(zone_is_internet(&decode_zone_stream(&three)));
        let two = utf16le_bom("[ZoneTransfer]\r\nZoneId=2\r\n");
        assert!(!zone_is_internet(&decode_zone_stream(&two)));
        // 홀수 바이트 꼬리·BOM 만 있어도 패닉 없이 false
        assert!(!zone_is_internet(&decode_zone_stream(&[0xFF, 0xFE, 0x5A])));
        assert!(!zone_is_internet(&decode_zone_stream(&[0xFF, 0xFE])));
        // UTF-8 BOM 으로 시작하는 첫 줄
        assert!(zone_is_internet(&decode_zone_stream(
            b"\xEF\xBB\xBFZoneId=3\n"
        )));
    }

    /// 임시 폴더의 파일에 `:Zone.Identifier` 스트림을 써서 실제 NTFS 대체 데이터 스트림 경로로 판정한다.
    /// 스트림을 쓸 수 없는 파일 시스템(NTFS 가 아닌 곳)이면 건너뛴다.
    #[test]
    fn 실제_대체_데이터_스트림으로_판정한다() {
        let dir = scratch("ads");
        let file = dir.join("받은 파일 #1.xlsx");
        fs::write(&file, b"data").unwrap();
        let path = file.to_str().unwrap();
        let stream = format!("{path}:Zone.Identifier");
        // 스트림이 없는 로컬 파일
        assert!(!is_from_internet(path));
        if fs::write(&stream, b"[ZoneTransfer]\r\nZoneId=3\r\n").is_err() {
            eprintln!("NTFS 대체 데이터 스트림을 쓸 수 없어 건너뛴다");
            let _ = fs::remove_dir_all(&dir);
            return;
        }
        assert!(is_from_internet(path));
        // 본문은 스트림과 따로다
        assert_eq!(fs::read(&file).unwrap(), b"data");
        let cases: [(Vec<u8>, bool); 4] = [
            (b"[ZoneTransfer]\r\nZoneId=2\r\n".to_vec(), false),
            (
                b"\xEF\xBB\xBF[ZoneTransfer]\r\nZoneId=3;\r\n".to_vec(),
                true,
            ),
            (
                b"[ZoneTransfer]\r\nReferrerUrl=\xFF\r\nZoneId=4\r\n".to_vec(),
                true,
            ),
            (utf16le_bom("[ZoneTransfer]\r\nZoneId=3\r\n"), true),
        ];
        for (bytes, expected) in cases {
            fs::write(&stream, &bytes).unwrap();
            assert_eq!(is_from_internet(path), expected, "{bytes:?}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

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
