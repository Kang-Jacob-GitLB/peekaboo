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
}
