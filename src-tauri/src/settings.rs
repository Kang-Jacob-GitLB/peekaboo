// 설정. Electron 의 src/main/settings.ts 와 같은 파일(`settings.json`)·필드·기본값 병합 규칙이다.
// 릴리스는 Electron 과 같은 `%APPDATA%\Peekaboo\settings.json` 을 읽고 써서 전환 뒤에도 설정이 이어진다.
//
// 병합 규칙: 기본값 위에 파일의 값을 덮는다(`{ ...defaults, ...JSON.parse(file) }`). 파일을 못 읽거나 JSON 이 아니면 기본값.
// Electron 은 모르는 키를 그대로 들고 있다가 다시 쓰므로, 여기서도 모르는 키를 보존한다(Electron·Tauri 를 오가도 값이 사라지지 않게).
// 키 순서도 Electron 과 같다: 기본 필드가 앞, 모르는 키는 파일에 있던 순서(serde_json 의 `preserve_order` 로 맡는다).
// 저장은 같은 폴더의 임시 파일에 쓴 뒤 rename 으로 바꿔치기해, 쓰는 도중 꺼져도 settings.json 이 반쯤 쓰인 채 남지 않게 한다.
// 타입이 틀린 필드는 Electron 과 달리 기본값으로 되돌린다(Rust 쪽 코드가 타입을 믿을 수 있게).
// `keepWarmSeconds` 는 Tauri 가 쓰지 않지만(spare 창을 이식하지 않았다) 왕복 보존한다.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Map, Number, Value};

const HOTKEY_ENABLED: &str = "hotkeyEnabled";
const KEEP_WARM_SECONDS: &str = "keepWarmSeconds";

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// 스페이스바 단축키 사용
    pub hotkey_enabled: bool,
    pub keep_warm_seconds: f64,
    /// 이 앱이 모르는 키(다시 쓸 때 그대로 둔다)
    extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hotkey_enabled: true,
            keep_warm_seconds: 0.0,
            extra: Map::new(),
        }
    }
}

impl Settings {
    /// 파일 내용을 해석한다. 깨졌으면 기본값.
    pub fn parse(text: &str) -> Settings {
        let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(text) else {
            return Settings::default();
        };
        let defaults = Settings::default();
        // preserve_order 에서 `remove` 는 swap_remove 라 남은 키의 순서를 흐트러뜨린다 — shift_remove 를 쓴다
        let hotkey_enabled = match map.shift_remove(HOTKEY_ENABLED) {
            Some(Value::Bool(b)) => b,
            _ => defaults.hotkey_enabled,
        };
        let keep_warm_seconds = match map.shift_remove(KEEP_WARM_SECONDS) {
            Some(Value::Number(n)) => n.as_f64().unwrap_or(defaults.keep_warm_seconds),
            _ => defaults.keep_warm_seconds,
        };
        Settings {
            hotkey_enabled,
            keep_warm_seconds,
            extra: map,
        }
    }

    /// 파일에 쓸 내용(`JSON.stringify(settings, null, 2)` 와 같은 들여쓰기). 기본 필드가 앞, 모르는 키는 읽은 순서.
    pub fn to_json(&self) -> String {
        let mut map = Map::new();
        map.insert(HOTKEY_ENABLED.into(), Value::Bool(self.hotkey_enabled));
        map.insert(KEEP_WARM_SECONDS.into(), number(self.keep_warm_seconds));
        for (k, v) in &self.extra {
            map.insert(k.clone(), v.clone());
        }
        serde_json::to_string_pretty(&Value::Object(map)).expect("설정 직렬화는 실패하지 않는다")
    }
}

/// 정수로 떨어지면 정수로 쓴다(JS 의 `0` 이 `0.0` 으로 바뀌지 않게)
fn number(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9e15 {
        Value::Number(Number::from(v as i64))
    } else {
        Number::from_f64(v).map_or(Value::Null, Value::Number)
    }
}

/// 파일과 짝지은 현재 설정. 앱 상태(`manage`)로 둔다.
pub struct SettingsStore {
    path: Option<PathBuf>,
    current: Mutex<Settings>,
}

impl SettingsStore {
    /// 파일을 읽어 연다. `path` 가 None(데이터 폴더를 모름)이면 기본값을 메모리에서만 쓴다.
    pub fn load(path: Option<PathBuf>) -> Self {
        let current = path
            .as_deref()
            .and_then(|p| fs::read_to_string(p).ok())
            .map_or_else(Settings::default, |text| Settings::parse(&text));
        SettingsStore {
            path,
            current: Mutex::new(current),
        }
    }

    pub fn get(&self) -> Settings {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 단축키 설정을 바꾸고 저장한다
    pub fn set_hotkey_enabled(&self, enabled: bool) -> Settings {
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        current.hotkey_enabled = enabled;
        if let Some(path) = &self.path {
            if let Err(e) = save(path, &current) {
                log_error!("failed to save settings: {e}");
            }
        }
        current.clone()
    }
}

fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // 같은 폴더(같은 볼륨)에 임시 파일을 만들어야 rename 이 원자적이다. Rust 의 fs::rename 은 Windows 에서도 대상을 덮어쓴다.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let result = fs::write(&tmp, settings.to_json()).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("peekaboo-settings-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("settings.json")
    }

    #[test]
    fn 기본값() {
        let d = Settings::default();
        assert!(d.hotkey_enabled);
        assert_eq!(d.keep_warm_seconds, 0.0);
    }

    #[test]
    fn 파일_값이_기본값을_덮는다() {
        let s = Settings::parse(r#"{"hotkeyEnabled": false}"#);
        assert!(!s.hotkey_enabled);
        assert_eq!(s.keep_warm_seconds, 0.0);
        let s = Settings::parse(r#"{"keepWarmSeconds": 30}"#);
        assert!(s.hotkey_enabled);
        assert_eq!(s.keep_warm_seconds, 30.0);
    }

    #[test]
    fn 깨진_파일이나_객체가_아니면_기본값() {
        for text in ["", "not json", "[1,2]", "5", "null", "\u{feff}{}"] {
            assert_eq!(Settings::parse(text), Settings::default(), "{text:?}");
        }
    }

    #[test]
    fn 타입이_틀린_필드는_기본값으로() {
        let s = Settings::parse(r#"{"hotkeyEnabled": "no", "keepWarmSeconds": "x"}"#);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn 모르는_키는_다시_쓸_때_보존된다() {
        let s = Settings::parse(r#"{"hotkeyEnabled": false, "future": {"a": [1, 2]}, "z": 1}"#);
        let again = Settings::parse(&s.to_json());
        assert_eq!(again, s);
        assert!(s.to_json().contains("\"future\""));
    }

    #[test]
    fn 키_순서는_기본_필드가_앞_나머지는_읽은_순서() {
        // 파일에서는 keepWarmSeconds 가 앞이고 모르는 키가 알파벳 역순이어도, 쓸 때는 기본 필드(hotkeyEnabled, keepWarmSeconds)가 앞
        let s = Settings::parse(
            r#"{"z": 1, "keepWarmSeconds": 5, "m": 2, "hotkeyEnabled": false, "a": 3}"#,
        );
        let json = s.to_json();
        let keys: Vec<&str> = ["hotkeyEnabled", "keepWarmSeconds", "z", "m", "a"]
            .into_iter()
            .collect();
        let positions: Vec<usize> = keys
            .iter()
            .map(|k| json.find(&format!("\"{k}\"")).unwrap())
            .collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{json}");
        // 다시 읽어 써도 같다
        assert_eq!(Settings::parse(&json).to_json(), json);
    }

    #[test]
    fn 직렬화는_electron_과_같은_모양() {
        assert_eq!(
            Settings::default().to_json(),
            "{\n  \"hotkeyEnabled\": true,\n  \"keepWarmSeconds\": 0\n}"
        );
        let s = Settings::parse(r#"{"keepWarmSeconds": 2.5}"#);
        assert!(s.to_json().contains("\"keepWarmSeconds\": 2.5"));
    }

    #[test]
    fn 저장하고_다시_읽는다() {
        let path = temp_file("roundtrip");
        let store = SettingsStore::load(Some(path.clone()));
        assert!(store.get().hotkey_enabled);
        store.set_hotkey_enabled(false);
        assert!(!SettingsStore::load(Some(path.clone())).get().hotkey_enabled);
        store.set_hotkey_enabled(true);
        assert!(SettingsStore::load(Some(path.clone())).get().hotkey_enabled);
        // 임시 파일이 남지 않는다
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["settings.json"]);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn 저장_실패해도_기존_파일은_그대로() {
        let path = temp_file("atomic");
        let store = SettingsStore::load(Some(path.clone()));
        store.set_hotkey_enabled(false);
        let before = fs::read_to_string(&path).unwrap();
        // 임시 파일 자리를 폴더로 막아 쓰기를 실패시킨다
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(format!(".{}.tmp", std::process::id()));
        fs::create_dir(&tmp).unwrap();
        assert!(save(&path, &Settings::default()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn 경로가_없으면_메모리에서만() {
        let store = SettingsStore::load(None);
        store.set_hotkey_enabled(false);
        assert!(!store.get().hotkey_enabled);
    }
}
