// native 모드(?native)에서만 애니메이션 요소를 켠다. CSP 가 인라인 스크립트를 막아 별도 파일로 둔다.
if (location.search.includes('native')) document.body.classList.add('native')
