// vite 의 `?url` import (번들 내 로컬 파일 URL)
declare module '*?url' {
  const url: string
  export default url
}
