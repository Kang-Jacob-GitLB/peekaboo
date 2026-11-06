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
| `key` | `key`: `space`·`esc`·`enter`·`up`·`down`·`left`·`right`·`copy`, `consumed` | 탐색기/바탕화면 파일 뷰(또는 `hostPid` 의 미리보기 창)가 포커스일 때 관련 키 입력. `consumed=true` 면 헬퍼가 키를 삼켰음 |
| `selection` | `id?`, `paths[]`, `focused`, `source`: `explorer`·`desktop`, `hwnd` | `getSelection` 응답(`id` 있음) 또는 선택 변경 알림(`id` 없음) |
| `pong` | `id` | `ping` 응답 |
| `error` | `id?`, `code`, `message` | 복구 가능한 오류. `ELEVATED`(관리자 권한 창), `NO_VIEW`, `COM_FAIL`, `BAD_MESSAGE`, `INTERNAL` |
| `nativePreview` | `id`, `status`: `shown`·`unavailable`·`failed`, `message?` | `nativePreview.open` 응답. `unavailable` = 등록된 미리보기 핸들러 없음 |
| `foreground` | `pid`, `shell` | 포그라운드 창이 바뀜. `shell` = 셸(탐색기 창·바탕화면·작업 표시줄 등 explorer.exe) 창인지. Electron 은 미리보기를 닫지 않고(다른 앱으로 전환해도 떠 있게 둔다), `shell=false` 인 동안만 미리보기 창이 클릭으로 포커스를 받게 한다(탐색기가 포그라운드일 때는 탐색기 포커스 유지). 셸 창끼리의 전환(탐색기 A → B)도 알린다. 헬퍼 자신의 창은 알리지 않는다. 시작 메뉴·검색·알림 센터·화면 캡처처럼 explorer.exe 밖에서 도는 셸 화면(`Windows.UI.Core.CoreWindow`)도 셸로 본다 |

## Electron → Helper

| type | 필드 | 설명 |
|---|---|---|
| `config` | `enabled`, `hostPid?` | 단축키 on/off. `false` 면 어떤 키도 가로채지 않는다. `hostPid` = 미리보기 창을 띄우는 Electron 프로세스 — 미리보기가 열려 있고 그 프로세스의 창이 포그라운드면(다른 앱에 있다가 미리보기를 클릭한 경우) Space·Esc·Ctrl+C(텍스트 선택 시)를 삼키고 `key` 로 알린다 |
| `state` | `previewOpen` | 미리보기 열림 여부. 열려 있을 때만 Esc 를 삼키고 선택 변경 감시를 켠다 |
| `textSelection` | `selected` | 미리보기 창에 텍스트가 선택돼 있는지. `true` 인 동안만 Ctrl+C 를 삼킨다. `state{previewOpen:false}` 를 받으면 헬퍼가 `false` 로 되돌린다 |
| `getSelection` | `id` | 포그라운드 탐색기/바탕화면의 현재 선택 조회 |
| `ping` | `id` | 생존 확인. Electron 은 2초 간격으로 보내고 연속 3회 무응답이면 헬퍼를 재시작한다 |
| `shutdown` | — | 정상 종료 요청 |
| `nativePreview.open` | `id`, `path`, `hwnd`, `rect`{`x`,`y`,`width`,`height`}, `dark` | Windows 미리보기 핸들러(IPreviewHandler)를 미리보기 창(`hwnd`) 안 `rect`(클라이언트 영역, 물리 픽셀)에 띄운다. 이전 것은 자동으로 닫힌다 |
| `nativePreview.resize` | `id`, `rect` | 표시 영역 변경 |
| `nativePreview.close` | `id` | 닫기(Unload) |

### Windows 미리보기 핸들러 호스팅

- 핸들러는 `AssocQueryString(ASSOCSTR_SHELLEXTENSION, ".ext", "{8895b1c6-b41f-4c1c-a562-0d564250836f}")` 로 찾고 `CLSCTX_LOCAL_SERVER` 로만 만든다(핸들러는 항상 별도 프로세스에서 실행).
- 초기화는 `IInitializeWithStream` → `IInitializeWithItem` → `IInitializeWithFile` 순으로 시도한다(공유 읽기, 파일을 잠그지 않음).
- 헬퍼가 `rect` 위치에 컨테이너 자식 창을 만들고, 핸들러에는 그 안의 `(0,0,w,h)` 를 넘긴다. 핸들러마다 RECT 위치 해석이 달라서다(실측: PowerPoint 는 위치를 무시).
- 핸들러 호출은 전용 STA 스레드에서 하므로 `DoPreview` 가 오래 걸려도 선택 조회는 막히지 않는다. 대기열의 오래된 `open` 은 건너뛴다.
- Electron 은 8초 안에 응답이 없으면 `failed` 로 보고 렌더러의 자체 미리보기로 넘어간다.

## 키 처리 규칙

헬퍼가 키를 "가로챌 수 있는" 조건: 단축키 활성 + 포그라운드가 탐색기(`CabinetWClass`) 또는 바탕화면(`Progman`/`WorkerW`) + 키보드 포커스가 파일 뷰(`SHELLDLL_DefView` 하위 `DirectUIHWND`, 바탕화면은 `SysListView32`) + 캐럿 없음 + 수정키(Ctrl/Alt/Win) 미입력(Ctrl+C 만 예외: Ctrl 단독 조합일 때만 다룬다).

| 키 | 미리보기 닫힘 | 미리보기 열림 |
|---|---|---|
| Space | 삼킴 → `key` 전송 (Electron 이 `getSelection` 후 열기) | 삼킴 → `key` 전송 (닫기) |
| Esc | 통과 | 삼킴 → `key` 전송 (닫기) |
| 방향키 | 통과 | 통과(탐색기가 선택 이동) + `key(consumed=false)` 전송, 선택 변경은 `selection` 알림으로 |
| Enter | 통과 | 통과(탐색기가 기본 앱으로 열기) + `key(consumed=false)` 전송 → Electron 이 미리보기 닫기 |
| Ctrl+C | 통과(탐색기 파일 복사) | `textSelection=true` 면 삼킴 → `key(copy)` 전송 (Electron 이 선택한 텍스트를 클립보드에 쓰기), 아니면 통과 |

## 예시

```jsonl
{"v":1,"type":"ready","version":"0.1.0","pid":1234}
{"v":1,"type":"key","key":"space","consumed":true}
{"v":1,"type":"getSelection","id":7}
{"v":1,"type":"selection","id":7,"paths":["C:\\Users\\me\\a.png"],"focused":"C:\\Users\\me\\a.png","source":"explorer","hwnd":329874}
{"v":1,"type":"error","id":8,"code":"ELEVATED","message":"Foreground window belongs to an elevated process"}
```
