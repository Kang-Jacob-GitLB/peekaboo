// 클립보드에 텍스트를 쓴다(Electron 의 clipboard.writeText). Win32 를 직접 부르므로 새 크레이트가 없다.
// 렌더러는 클립보드에 접근하지 못한다: 우클릭·Ctrl+C 복사는 호스트(Rust)가 이 함수로만 쓴다(preview.rs 의 copy_to_clipboard).
//
// 절차(Microsoft Learn "Using the Clipboard" 의 "Copying Information to the Clipboard"):
// OpenClipboard → EmptyClipboard → GlobalAlloc(GMEM_MOVEABLE) → GlobalLock 으로 UTF-16 을 채우고 GlobalUnlock →
// SetClipboardData(CF_UNICODETEXT, hMem) → CloseClipboard. SetClipboardData 가 성공하면 메모리 소유권은 시스템으로 넘어가므로
// 실패했을 때만 GlobalFree 한다.
//
// 소유 창: `OpenClipboard(NULL)` 로 열면 이어지는 EmptyClipboard 가 클립보드 소유자를 NULL 로 만들고, 그러면 SetClipboardData 가
// 실패할 수 있다(Learn OpenClipboard·EmptyClipboard 의 Remarks). 그래서 미리보기 창의 HWND 를 소유 창으로 넘긴다.

use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

/// CF_UNICODETEXT(Standard Clipboard Formats: 13)
const CF_UNICODETEXT: u32 = 13;
/// 다른 프로세스(클립보드 기록기 등)가 클립보드를 잠깐 잡고 있으면 OpenClipboard 가 실패한다
const OPEN_ATTEMPTS: u32 = 10;
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(10);

/// 클립보드에 쓸 UTF-16(끝에 NUL 하나). CF_UNICODETEXT 는 NUL 로 끝나는 문자열이다.
fn utf16_with_nul(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `owner` 는 클립보드를 열 때 소유 창으로 쓸 HWND(미리보기 창, 호출한 스레드의 창이어야 한다).
pub fn set_text(text: &str, owner: isize) -> Result<(), String> {
    let wide = utf16_with_nul(text);
    let owner = HWND(owner as *mut std::ffi::c_void);
    let bytes = wide.len() * std::mem::size_of::<u16>();
    // SAFETY: 아래 Win32 호출은 모두 이 함수 안에서 짝을 맞춰 쓰고, 잠근 메모리는 bytes 만큼만 쓴다.
    unsafe {
        let mut opened = Err("OpenClipboard".to_owned());
        for attempt in 0..OPEN_ATTEMPTS {
            match OpenClipboard(Some(owner)) {
                Ok(()) => {
                    opened = Ok(());
                    break;
                }
                Err(e) => opened = Err(format!("OpenClipboard: {e}")),
            }
            if attempt + 1 < OPEN_ATTEMPTS {
                std::thread::sleep(OPEN_RETRY_DELAY);
            }
        }
        opened?;
        let result = write_open(&wide, bytes);
        // 닫기에 실패해도 쓰기 결과를 우선한다
        let _ = CloseClipboard();
        result
    }
}

/// 클립보드가 열린 상태에서 비우고 쓴다.
///
/// # Safety
/// `OpenClipboard` 가 성공한 뒤에만 부른다. `bytes` 는 `wide` 의 바이트 길이여야 한다.
unsafe fn write_open(wide: &[u16], bytes: usize) -> Result<(), String> {
    EmptyClipboard().map_err(|e| format!("EmptyClipboard: {e}"))?;
    let mem = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| format!("GlobalAlloc: {e}"))?;
    let ptr = GlobalLock(mem);
    if ptr.is_null() {
        let _ = GlobalFree(Some(mem));
        return Err("GlobalLock".into());
    }
    std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr.cast::<u16>(), wide.len());
    // 마지막 잠금 해제는 "잠금 수가 0 이 됨"을 오류로 돌려줄 수 있어 결과를 보지 않는다
    let _ = GlobalUnlock(mem);
    let handle = HANDLE(mem.0);
    match SetClipboardData(CF_UNICODETEXT, Some(handle)) {
        Ok(_) => Ok(()),
        Err(e) => {
            // 소유권이 넘어가지 않았으므로 직접 해제한다
            let _ = GlobalFree(Some(HGLOBAL(mem.0)));
            Err(format!("SetClipboardData: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_는_nul_로_끝나고_서로게이트를_쓴다() {
        assert_eq!(utf16_with_nul(""), vec![0]);
        assert_eq!(utf16_with_nul("a가"), vec![0x61, 0xAC00, 0]);
        // U+1F600 → D83D DE00
        assert_eq!(utf16_with_nul("😀"), vec![0xD83D, 0xDE00, 0]);
    }
}
