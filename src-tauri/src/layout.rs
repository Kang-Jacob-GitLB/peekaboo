// 미리보기 창 크기 계산. Electron 의 src/main/preview-window.ts(fit·position)와 같은 식이다.
// 상수와 계산은 DIP(논리 픽셀) 기준이고, Tauri 가 쓰는 물리 픽셀과의 환산은 호출부(preview.rs)가 scale_factor 로 한다.

use serde::Deserialize;

/// 내용 크기에 맞출 때: 창 테두리(1px×2)와 헤더(38px + 테두리)
pub const CHROME_W: f64 = 2.0;
pub const CHROME_H: f64 = 40.0;
/// 작은 사진도 헤더(파일명·크기)가 읽히도록 하는 최소 내용 크기
pub const MIN_CONTENT_W: f64 = 420.0;
pub const MIN_CONTENT_H: f64 = 240.0;
/// 내용에 맞출 때 작업 영역에서 차지할 수 있는 최대 비율
pub const MAX_AREA_RATIO: f64 = 0.85;

/// 렌더러가 알려 주는 내용 크기(CSS px). `PeekabooApi.fit` 의 `ContentSize` 와 같다.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct ContentSize {
    pub width: f64,
    pub height: f64,
    /// true 면 화면 한도까지 키운다(문서처럼 비율만 의미 있는 경우)
    #[serde(default)]
    pub grow: bool,
}

impl ContentSize {
    /// 둘 다 유한한 양수여야 한다(렌더러가 보낸 값을 믿지 않는다).
    pub fn is_valid(&self) -> bool {
        [self.width, self.height]
            .iter()
            .all(|n| n.is_finite() && *n > 0.0)
    }
}

/// 내용 비율에 맞춘 창 크기(DIP). 작업 영역의 85% 안에 들어가게 줄이고, grow 가 아니면 원래 크기보다 키우지 않는다.
/// 너무 작으면 최소 크기까지 늘린다(이때는 비율이 달라진다).
pub fn fit_size(area_w: f64, area_h: f64, size: &ContentSize) -> (f64, f64) {
    let max_w = (area_w * MAX_AREA_RATIO).floor() - CHROME_W;
    let max_h = (area_h * MAX_AREA_RATIO).floor() - CHROME_H;
    let mut scale = (max_w / size.width).min(max_h / size.height);
    if !size.grow {
        scale = scale.min(1.0);
    }
    let width = (size.width * scale).round().max(MIN_CONTENT_W) + CHROME_W;
    let height = (size.height * scale).round().max(MIN_CONTENT_H) + CHROME_H;
    (width, height)
}

/// 기본 크기(DIP): 작업 영역의 70%, 최대 1280×900
pub fn default_size(area_w: f64, area_h: f64) -> (f64, f64) {
    (
        (area_w * 0.7).round().min(1280.0),
        (area_h * 0.7).round().min(900.0),
    )
}

/// 화면 좌표(물리 픽셀)의 사각형. 모니터·작업 영역·창 모두 이것으로 다룬다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// 맞닿기만 하는 경우는 겹침이 아니다.
    pub fn intersects(&self, other: &Rect) -> bool {
        let (ax2, ay2) = (
            i64::from(self.x) + i64::from(self.width),
            i64::from(self.y) + i64::from(self.height),
        );
        let (bx2, by2) = (
            i64::from(other.x) + i64::from(other.width),
            i64::from(other.y) + i64::from(other.height),
        );
        i64::from(self.x) < bx2
            && i64::from(other.x) < ax2
            && i64::from(self.y) < by2
            && i64::from(other.y) < ay2
    }
}

/// 화면 밖 시험 모드(`PEEKABOO_OFFSCREEN=1`)에서 창을 둘 위치. 모든 모니터의 왼쪽 바깥이다.
/// 어느 모니터도 그 왼쪽에는 없으므로 창이 어느 화면에도 걸치지 않는다. -32000 은 Windows 가 최소화 창에 쓰는 좌표라 피한다.
pub fn offscreen_origin(monitors: &[Rect], width: i32) -> (i32, i32) {
    const MARGIN: i32 = 200;
    let min_x = monitors.iter().map(|m| m.x).min().unwrap_or(0);
    let min_y = monitors.iter().map(|m| m.y).min().unwrap_or(0);
    (min_x - width - MARGIN, min_y)
}

/// 창이 어느 모니터와든 걸치는지(화면 밖 모드의 자가 검사: 걸치면 띄우지 않는다)
pub fn overlaps_any(window: &Rect, monitors: &[Rect]) -> bool {
    monitors.iter().any(|m| window.intersects(m))
}

/// 창의 왼쪽 위 좌표. 기본은 작업 영역 중앙(창이 더 크면 왼쪽 위)이다.
/// `moved_from`(사용자가 헤더를 끌어 옮긴 뒤의 지금 창 위치)이 있으면 그 중심을 유지하되 작업 영역 밖으로 나가지 않게 한다
/// (Electron 의 PreviewWindow.place 와 같다).
pub fn place_origin(area: &Rect, width: i32, height: i32, moved_from: Option<&Rect>) -> (i32, i32) {
    match moved_from {
        None => (
            area.x + (f64::from(area.width - width) / 2.0).max(0.0).round() as i32,
            area.y + (f64::from(area.height - height) / 2.0).max(0.0).round() as i32,
        ),
        Some(b) => {
            // JS 의 Math.round 처럼 .5 는 위로
            let round = |v: f64| (v + 0.5).floor() as i32;
            let x = round(f64::from(b.x) + f64::from(b.width - width) / 2.0);
            let y = round(f64::from(b.y) + f64::from(b.height - height) / 2.0);
            (
                clamp(x, area.x, area.x + area.width - width),
                clamp(y, area.y, area.y + area.height - height),
            )
        }
    }
}

/// min 이 max 보다 크면(창이 작업 영역보다 크면) min
fn clamp(v: i32, min: i32, max: i32) -> i32 {
    min.max(v.min(max))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: f64, height: f64, grow: bool) -> ContentSize {
        ContentSize {
            width,
            height,
            grow,
        }
    }

    #[test]
    fn 작은_사진은_원래_크기에_테두리만_더한다() {
        // 1920×1080 작업 영역, 800×600 사진 → 키우지 않는다
        assert_eq!(
            fit_size(1920.0, 1080.0, &size(800.0, 600.0, false)),
            (802.0, 640.0)
        );
    }

    #[test]
    fn 큰_사진은_작업_영역_85퍼센트_안으로_줄인다() {
        // max_w = floor(1632) - 2 = 1630, max_h = floor(918) - 40 = 878. 4000×3000 → scale = min(0.4075, 0.2927)
        // → 내용 1171×878(+테두리)
        assert_eq!(
            fit_size(1920.0, 1080.0, &size(4000.0, 3000.0, false)),
            (1173.0, 918.0)
        );
    }

    #[test]
    fn grow_면_작은_내용도_한도까지_키운다() {
        // 612×792(문서) → 높이 한도 878 에 맞춘다
        let (w, h) = fit_size(1920.0, 1080.0, &size(612.0, 792.0, true));
        assert_eq!(h, 878.0 + CHROME_H);
        assert_eq!(w, (612.0_f64 * 878.0 / 792.0).round() + CHROME_W);
    }

    #[test]
    fn 너무_작으면_최소_크기로_늘린다() {
        assert_eq!(
            fit_size(1920.0, 1080.0, &size(16.0, 16.0, false)),
            (MIN_CONTENT_W + CHROME_W, MIN_CONTENT_H + CHROME_H)
        );
    }

    #[test]
    fn 기본_크기는_70퍼센트이고_상한이_있다() {
        assert_eq!(default_size(1000.0, 800.0), (700.0, 560.0));
        assert_eq!(default_size(3840.0, 2160.0), (1280.0, 900.0));
    }

    #[test]
    fn 내용_크기_검증() {
        assert!(size(1.0, 1.0, false).is_valid());
        for (w, h) in [
            (0.0, 1.0),
            (1.0, -1.0),
            (f64::NAN, 1.0),
            (1.0, f64::INFINITY),
        ] {
            assert!(!size(w, h, false).is_valid(), "{w} {h}");
        }
    }

    #[test]
    fn 화면_밖_좌표는_어느_모니터와도_겹치지_않는다() {
        let monitors = [
            Rect::new(0, 0, 1920, 1080),
            Rect::new(-1280, -200, 1280, 1024),
            Rect::new(1920, 100, 2560, 1440),
        ];
        let (w, h) = (1200, 800);
        let (x, y) = offscreen_origin(&monitors, w);
        assert!(!overlaps_any(&Rect::new(x, y, w, h), &monitors));
        // 창이 아주 커도 마찬가지
        let (x, y) = offscreen_origin(&monitors, 9000);
        assert!(!overlaps_any(&Rect::new(x, y, 9000, 5000), &monitors));
        // 모니터를 모르면 원점 기준
        assert_eq!(offscreen_origin(&[], 100), (-300, 0));
    }

    #[test]
    fn 겹침_검사는_걸치는_창을_잡고_맞닿기만_하면_통과시킨다() {
        let m = [Rect::new(0, 0, 100, 100)];
        assert!(overlaps_any(&Rect::new(99, 99, 10, 10), &m));
        assert!(overlaps_any(&Rect::new(-5, -5, 10, 10), &m));
        assert!(overlaps_any(&Rect::new(10, 10, 5, 5), &m), "안에 들어간 창");
        assert!(!overlaps_any(&Rect::new(100, 0, 10, 10), &m));
        assert!(!overlaps_any(&Rect::new(-10, 0, 10, 10), &m));
        assert!(!overlaps_any(&Rect::new(0, 0, 10, 10), &[]));
    }

    #[test]
    fn 기본_배치는_작업_영역_중앙이고_크면_왼쪽_위() {
        let area = Rect::new(100, 50, 1000, 800);
        assert_eq!(place_origin(&area, 400, 200, None), (400, 350));
        assert_eq!(place_origin(&area, 1200, 900, None), (100, 50));
    }

    #[test]
    fn 사용자가_옮긴_창은_중심을_유지하고_작업_영역_안으로_가둔다() {
        let area = Rect::new(0, 0, 1000, 800);
        // 지금 창 (300,200) 400×300 → 새 크기 200×100: 중심 유지 (400,300)
        let cur = Rect::new(300, 200, 400, 300);
        assert_eq!(place_origin(&area, 200, 100, Some(&cur)), (400, 300));
        // 오른쪽 아래 구석에 있던 창이 커지면 작업 영역 안으로
        let cur = Rect::new(800, 600, 200, 200);
        assert_eq!(place_origin(&area, 600, 500, Some(&cur)), (400, 300));
        // 왼쪽 위로 삐져나간 창
        let cur = Rect::new(-50, -50, 100, 100);
        assert_eq!(place_origin(&area, 300, 300, Some(&cur)), (0, 0));
        // 작업 영역보다 큰 창은 min(왼쪽 위)
        let cur = Rect::new(10, 10, 100, 100);
        assert_eq!(place_origin(&area, 1200, 900, Some(&cur)), (0, 0));
    }
}
