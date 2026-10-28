# Helper ↔ Electron IPC 스펙 (v1)

- 전송: 헬퍼 프로세스의 stdin / stdout, **UTF-8 JSON Lines** (한 줄 = 한 객체, `\n` 구분)
- stdout 에는 프로토콜 메시지만 쓴다. 진단 로그는 stderr 로 내보내며 Electron 이 로그 파일에 합친다.
- 모든 메시지는 `v`(프로토콜 버전, 현재 1)와 `type` 을 가진다. 요청/응답 짝은 `id`(정수)로 맞춘다.
- 알 수 없는 `type` 은 무시한다(전방 호환). 파싱 실패 시 헬퍼는 `error{code:"BAD_MESSAGE"}` 를 보내고 계속 동작한다.

TypeScript 정의: `src/shared/ipc-types.ts` / C# 정의: `helper/Ipc/Messages.cs`

## Helper → Electron

| type | 필드 | 설명 |
|---|---|---|
| `ready` | `version`, `pid` | 키보드 훅 설치 완료 |
| `key` | `key`: `space`·`esc`·`enter`·`up`·`down`·`left`·`right`, `consumed` | 탐색기/바탕화면 파일 뷰가 포커스일 때 관련 키 입력. `consumed=true` 면 헬퍼가 키를 삼켰음 |
| `selection` | `id?`, `paths[]`, `focused`, `source`: `explorer`·`desktop`, `hwnd` | `getSelection` 응답(`id` 있음) 또는 선택 변경 알림(`id` 없음) |
| `pong` | `id` | `ping` 응답 |
| `error` | `id?`, `code`, `message` | 복구 가능한 오류. `ELEVATED`(관리자 권한 창), `NO_VIEW`, `COM_FAIL`, `BAD_MESSAGE`, `INTERNAL` |

## Electron → Helper

| type | 필드 | 설명 |
|---|---|---|
| `config` | `enabled` | 단축키 on/off. `false` 면 어떤 키도 가로채지 않는다 |
| `state` | `previewOpen` | 미리보기 열림 여부. 열려 있을 때만 Esc 를 삼키고 선택 변경 감시를 켠다 |
| `getSelection` | `id` | 포그라운드 탐색기/바탕화면의 현재 선택 조회 |
| `ping` | `id` | 생존 확인. Electron 은 2초 간격으로 보내고 연속 3회 무응답이면 헬퍼를 재시작한다 |
| `shutdown` | — | 정상 종료 요청 |

## 키 처리 규칙

헬퍼가 키를 "가로챌 수 있는" 조건: 단축키 활성 + 포그라운드가 탐색기(`CabinetWClass`) 또는 바탕화면(`Progman`/`WorkerW`) + 키보드 포커스가 파일 뷰(`SHELLDLL_DefView` 하위 `DirectUIHWND`) + 캐럿 없음 + 수정키(Ctrl/Alt/Win) 미입력.

| 키 | 미리보기 닫힘 | 미리보기 열림 |
|---|---|---|
| Space | 삼킴 → `key` 전송 (Electron 이 `getSelection` 후 열기) | 삼킴 → `key` 전송 (닫기) |
| Esc | 통과 | 삼킴 → `key` 전송 (닫기) |
| 방향키 | 통과 | 통과(탐색기가 선택 이동) + `key(consumed=false)` 전송, 선택 변경은 `selection` 알림으로 |
| Enter | 통과 | 통과(탐색기가 기본 앱으로 열기) + `key(consumed=false)` 전송 → Electron 이 미리보기 닫기 |

## 예시

```jsonl
{"v":1,"type":"ready","version":"0.1.0","pid":1234}
{"v":1,"type":"key","key":"space","consumed":true}
{"v":1,"type":"getSelection","id":7}
{"v":1,"type":"selection","id":7,"paths":["C:\\Users\\me\\a.png"],"focused":"C:\\Users\\me\\a.png","source":"explorer","hwnd":329874}
{"v":1,"type":"error","id":8,"code":"ELEVATED","message":"Foreground window belongs to an elevated process"}
```
