// 골든 비교용 픽스처 생성기. 결정적이라 매번 같은 바이트가 나온다.
// zip·tar 를 직접 조립하므로 실제 압축 도구가 만들지 않는 경계 입력(위장 Unicode Path, SFX, 잘린 파일 등)도 만들 수 있다.
//
// test/fixtures/expected/*.json 은 이 파일의 픽스처를 이 PR 이 지우는 Node 구현(88fd827 의
// src/main/file-service.ts·tar-list.ts)에 TZ=UTC 로 돌려 얻은 출력이다. 렌더러 구현이 그 JSON 과 같아야 한다.

import { crc32, deflateRawSync, gzipSync } from 'node:zlib'

// ---------- 공통 ----------

const u16 = (v: number): Buffer => {
  const b = Buffer.alloc(2)
  b.writeUInt16LE(v)
  return b
}
const u32 = (v: number): Buffer => {
  const b = Buffer.alloc(4)
  b.writeUInt32LE(v)
  return b
}
const u64 = (v: number): Buffer => {
  const b = Buffer.alloc(8)
  b.writeBigUInt64LE(BigInt(v))
  return b
}

// "한글 테스트" 등을 CP949(EUC-KR)로 인코딩한 바이트(TextEncoder 는 euc-kr 인코딩을 지원하지 않는다)
export const CP949 = {
  hangul: Buffer.from([0xc7, 0xd1, 0xb1, 0xdb]), // 한글
  test: Buffer.from([0xc5, 0xd7, 0xbd, 0xba, 0xc6, 0xae]) // 테스트
}

// ---------- zip ----------

interface ZipEntry {
  name: string | Buffer
  data?: Buffer
  method?: 0 | 8
  flags?: number
  /** [date, time] MS-DOS 값. 기본 2020-06-01 12:00:00 */
  dos?: [number, number]
  extra?: Buffer[]
  /** 중앙 디렉터리에 적을 크기를 덮어쓴다(zip64 포화값 등) */
  rawSize?: number
  compSize?: number
}

interface ZipOptions {
  comment?: string | Buffer
  zip64?: boolean
  /** 앞에 붙일 바이트(자체 압축 풀기 exe 처럼). 오프셋은 보정하지 않는다 */
  prefix?: Buffer
}

export const field = (id: number, data: Buffer): Buffer => Buffer.concat([u16(id), u16(data.length), data])
const extTime = (sec: number): Buffer => field(0x5455, Buffer.concat([Buffer.from([1]), u32(sec)]))
const unicodePath = (nameBytes: Buffer, shown: string, crc = crc32(nameBytes)): Buffer =>
  field(0x7075, Buffer.concat([Buffer.from([1]), u32(crc), Buffer.from(shown, 'utf8')]))

const DOS_2020_06_01_12 = [((2020 - 1980) << 9) | (6 << 5) | 1, 12 << 11] as [number, number]

export function buildZip(entries: ZipEntry[], opts: ZipOptions = {}): Buffer {
  const parts: Buffer[] = []
  const cd: Buffer[] = []
  let off = 0
  for (const e of entries) {
    const nameBytes = typeof e.name === 'string' ? Buffer.from(e.name, 'utf8') : e.name
    const data = e.data ?? Buffer.alloc(0)
    const method = e.method ?? 0
    const body = method === 8 ? deflateRawSync(data) : data
    const flags = e.flags ?? 0
    const [dd, dt] = e.dos ?? DOS_2020_06_01_12
    const extra = Buffer.concat(e.extra ?? [])
    const rawSize = e.rawSize ?? data.length
    const compSize = e.compSize ?? body.length

    const local = Buffer.concat([
      u32(0x04034b50), u16(20), u16(flags), u16(method), u16(dt), u16(dd), u32(crc32(data)),
      u32(Math.min(body.length, 0xffffffff)), u32(Math.min(data.length, 0xffffffff)), u16(nameBytes.length), u16(0),
      nameBytes, body
    ])
    cd.push(Buffer.concat([
      u32(0x02014b50), u16(20), u16(20), u16(flags), u16(method), u16(dt), u16(dd), u32(crc32(data)),
      u32(compSize), u32(rawSize), u16(nameBytes.length), u16(extra.length), u16(0), u16(0), u16(0), u32(0),
      u32(off), nameBytes, extra
    ]))
    parts.push(local)
    off += local.length
  }
  const cdBuf = Buffer.concat(cd)
  const cdOff = off
  const commentBytes = opts.comment === undefined ? Buffer.alloc(0) : Buffer.from(opts.comment as string)
  const tail: Buffer[] = [cdBuf]
  if (opts.zip64) {
    const rec64Pos = cdOff + cdBuf.length
    tail.push(Buffer.concat([
      u32(0x06064b50), u64(44), u16(45), u16(45), u32(0), u32(0), u64(entries.length), u64(entries.length),
      u64(cdBuf.length), u64(cdOff)
    ]))
    tail.push(Buffer.concat([u32(0x07064b50), u32(0), u64(rec64Pos), u32(1)]))
    tail.push(Buffer.concat([
      u32(0x06054b50), u16(0), u16(0), u16(0xffff), u16(0xffff), u32(0xffffffff), u32(0xffffffff), u16(commentBytes.length), commentBytes
    ]))
  } else {
    tail.push(Buffer.concat([
      u32(0x06054b50), u16(0), u16(0), u16(entries.length), u16(entries.length), u32(cdBuf.length), u32(cdOff),
      u16(commentBytes.length), commentBytes
    ]))
  }
  return Buffer.concat([opts.prefix ?? Buffer.alloc(0), ...parts, ...tail])
}

const SAT = 0xffffffff

function zipFixtures(): Record<string, Buffer> {
  const text = Buffer.from('hello world\n'.repeat(50))
  const normal = buildZip(
    [
      { name: 'docs/', method: 0 },
      { name: 'docs/readme.txt', data: text, method: 8, extra: [extTime(1_600_000_000)] },
      { name: 'docs/한글.txt', data: text, method: 8, flags: 0x0800 },
      { name: 'win\\path\\backslash.txt', data: Buffer.from('x') },
      { name: 'secret.bin', data: Buffer.from('enc'), flags: 1 },
      { name: 'old.txt', data: Buffer.from('y'), dos: [0, 0] },
      { name: 'x'.repeat(1500) + '.txt', data: Buffer.from('z') }
    ],
    { comment: '  압축 파일 주석 \n' }
  )

  const zip64 = buildZip(
    [
      { name: 'huge.bin', rawSize: SAT, compSize: SAT, extra: [field(0x0001, Buffer.concat([u64(6_000_000_000), u64(5_000_000_000)]))] },
      { name: 'small.txt', data: Buffer.from('s') }
    ],
    { zip64: true }
  )

  // 이름이 CP949(UTF-8 플래그 없음), 또 하나는 UTF-8·CP949 어느 쪽도 아니라 latin1 로 폴백
  const cp949 = buildZip([
    { name: Buffer.concat([CP949.hangul, Buffer.from('.txt')]) },
    { name: Buffer.concat([Buffer.from('a'), Buffer.from([0x80, 0x9f]), Buffer.from('b.txt')]) },
    { name: Buffer.from('plain-utf8-한.txt', 'utf8') }
  ])

  // Unicode Path: 첫 항목은 CRC 가 맞아 채택, 둘째는 CRC 불일치(위장)라 원래 이름을 유지, 셋째는 버전 바이트가 2
  const evil = Buffer.from('evil.exe')
  const good = Buffer.from('good.dat')
  const ver2 = Buffer.from('ver2.dat')
  const unicode = buildZip([
    { name: good, extra: [unicodePath(good, '올바른.dat')] },
    { name: evil, extra: [unicodePath(evil, 'invoice.pdf', crc32(Buffer.from('other')))] },
    { name: ver2, extra: [field(0x7075, Buffer.concat([Buffer.from([2]), u32(crc32(ver2)), Buffer.from('v2.dat')]))] }
  ])

  const base = buildZip([{ name: 'a.txt', data: Buffer.from('aaa') }, { name: 'dir/b.txt', data: Buffer.from('bbb') }])
  const sfx = buildZip([{ name: 'a.txt', data: Buffer.from('aaa') }, { name: 'dir/b.txt', data: Buffer.from('bbb') }], {
    prefix: Buffer.alloc(1000, 0x4d)
  })

  // 항목 수가 많은 zip(max 로 자르는 경우)
  const many = buildZip(Array.from({ length: 60 }, (_, i) => ({ name: `f/${String(i).padStart(3, '0')}.txt`, data: Buffer.from('1') })))

  return {
    'normal.zip': normal,
    'zip64.zip': zip64,
    'cp949-names.zip': cp949,
    'unicode-path.zip': unicode,
    'sfx.zip': sfx,
    'many.zip': many,
    // EOCD 가 없다(끝이 잘림)
    'truncated-eocd.zip': base.subarray(0, base.length - 10),
    // 중앙 디렉터리 일부를 지워 cdOffset+cdSize 가 파일 크기를 넘는다
    'truncated-cd.zip': Buffer.concat([base.subarray(0, 40), base.subarray(base.length - 22)]),
    'tiny.zip': Buffer.from('PK'),
    'not-a.zip': Buffer.from('이것은 zip 이 아니라 그냥 텍스트입니다. '.repeat(5)),
    // 잘못된 CD 서명
    'bad-cd-signature.zip': (() => {
      const b = Buffer.from(base)
      const cdOff = b.readUInt32LE(b.length - 6)
      b.writeUInt32LE(0xdeadbeef, cdOff)
      return b
    })()
  }
}

// ---------- tar ----------

const BLOCK = 512

interface TarHeader {
  name?: string | Buffer
  size?: number
  type?: string
  mtime?: number
  prefix?: string
  /** "ustar" 매직을 넣는다 */
  ustar?: boolean
  sizeField?: Buffer
  sparseExtended?: boolean
}

const octal = (v: number, len: number): Buffer => Buffer.from(v.toString(8).padStart(len - 1, '0') + '\0', 'latin1')

export function tarHeader(h: TarHeader): Buffer {
  const b = Buffer.alloc(BLOCK)
  const name = typeof h.name === 'string' ? Buffer.from(h.name, 'utf8') : (h.name ?? Buffer.from('file.txt'))
  name.copy(b, 0, 0, Math.min(100, name.length))
  octal(0o644, 8).copy(b, 100)
  octal(0, 8).copy(b, 108)
  octal(0, 8).copy(b, 116)
  ;(h.sizeField ?? octal(h.size ?? 0, 12)).copy(b, 124)
  octal(h.mtime ?? 1_600_000_000, 12).copy(b, 136)
  b.fill(0x20, 148, 156)
  b[156] = (h.type ?? '0').charCodeAt(0)
  if (h.ustar) {
    Buffer.from('ustar\0', 'latin1').copy(b, 257)
    Buffer.from('00', 'latin1').copy(b, 263)
  }
  if (h.prefix) Buffer.from(h.prefix, 'utf8').copy(b, 345)
  if (h.sparseExtended) b[482] = 1
  let sum = 0
  for (let i = 0; i < BLOCK; i++) sum += b[i]
  Buffer.from(sum.toString(8).padStart(6, '0') + '\0 ', 'latin1').copy(b, 148)
  return b
}

const pad = (data: Buffer): Buffer => Buffer.concat([data, Buffer.alloc((BLOCK - (data.length % BLOCK)) % BLOCK)])

function tarFile(name: string | Buffer, body: Buffer | number, extra: Partial<TarHeader> = {}): Buffer {
  const data = typeof body === 'number' ? Buffer.alloc(body, 0x61) : body
  return Buffer.concat([tarHeader({ name, size: data.length, ...extra }), pad(data)])
}

/** pax 레코드 하나: "<길이> key=value\n" (길이는 자기 자신 포함) */
function paxRecord(key: string, value: string): Buffer {
  const body = Buffer.from(` ${key}=${value}\n`, 'utf8')
  let len = body.length + 1
  while (String(len).length + body.length !== len) len = String(len).length + body.length
  return Buffer.concat([Buffer.from(String(len)), body])
}

const END = Buffer.alloc(BLOCK * 2)

function tarFixtures(): Record<string, Buffer> {
  const plain = Buffer.concat([
    tarFile('dir/', 0, { type: '5' }),
    tarFile('dir/a.txt', 1000, { mtime: 1_700_000_000 }),
    tarFile('link', 0, { type: '2' }),
    tarFile('hard', 0, { type: '1' }),
    tarFile(Buffer.concat([CP949.hangul, Buffer.from('.txt')]), 10),
    tarFile('b.bin', 513),
    END
  ])

  const pax = Buffer.concat([
    // path·mtime·size 를 pax 로 덮어쓴다(size 는 실제 본문 길이와 같아야 다음 헤더에 닿는다)
    tarFile('PaxHeader', Buffer.concat([paxRecord('path', '긴/경로/pax-이름.txt'), paxRecord('mtime', '1700000000.5'), paxRecord('size', '700')]), { type: 'x' }),
    tarFile('short', 700),
    // 10진수가 아닌 pax size 는 무시하고 헤더 size 를 쓴다("1e3"·"0x200")
    tarFile('PaxHeader', Buffer.concat([paxRecord('size', '1e3'), paxRecord('path', 'bad-size-1e3.txt')]), { type: 'x' }),
    tarFile('ignored', 300),
    tarFile('PaxHeader', Buffer.concat([paxRecord('size', '0x200')]), { type: 'x' }),
    tarFile('bad-size-hex.txt', 100),
    // 전역 pax 는 목록에 쓰지 않는다
    tarFile('GlobalHead', paxRecord('comment', 'x'), { type: 'g' }),
    tarFile('after-global.txt', 5),
    // 길이 필드가 공백으로 시작하는 느슨한 레코드: 엄격 파싱이면 무시되어 이름이 헤더 값으로 남는다
    tarFile('PaxHeader', Buffer.from(' 20 path=loose-name\n', 'latin1'), { type: 'x' }),
    tarFile('loose-header-name.txt', 1),
    END
  ])

  const longName = '가'.repeat(60) + '/' + 'n'.repeat(120) + '.txt'
  const gnuLong = Buffer.concat([
    tarFile('././@LongLink', Buffer.concat([Buffer.from(longName, 'utf8'), Buffer.from([0])]), { type: 'L' }),
    tarFile('truncated-name-in-header', 20),
    tarFile('././@LongLink', Buffer.from('link-target\0'), { type: 'K' }),
    tarFile('sym', 0, { type: '2' }),
    // 이름이 1024 자를 넘는 항목
    tarFile('././@LongLink', Buffer.from('q'.repeat(1500) + '\0'), { type: 'L' }),
    tarFile('x', 1),
    // ustar prefix
    tarFile('leaf.txt', 7, { ustar: true, prefix: 'some/prefix/dir' }),
    END
  ])

  // 구형 GNU sparse: 확장 블록 2개가 이어진다(첫 번째 504 바이트 != 0, 두 번째 == 0)
  const ext = (more: boolean): Buffer => {
    const b = Buffer.alloc(BLOCK)
    if (more) b[504] = 1
    return b
  }
  const sparse = Buffer.concat([
    tarHeader({ name: 'sparse.dat', size: 1024, type: 'S', sparseExtended: true }),
    ext(true),
    ext(false),
    pad(Buffer.alloc(1024, 1)),
    tarFile('after-sparse.txt', 3),
    END
  ])
  // 확장 블록이 끝나지 않고 파일이 끊김
  const sparseBroken = Buffer.concat([tarHeader({ name: 'sparse.dat', size: 10, type: 'S', sparseExtended: true }), ext(true)])

  // 메타 블록(g)만 끝없이 이어지고 마지막에 파일이 하나 있는 파일
  const metaLoop = Buffer.concat([
    ...Array.from({ length: 500 }, () => tarFile('g', Buffer.alloc(0), { type: 'g' })),
    tarFile('real.txt', 4),
    END
  ])
  // 크기가 META_MAX(1MB)를 넘는 메타 블록
  const metaHuge = Buffer.concat([tarHeader({ name: 'x', size: 2 * 1024 * 1024, type: 'x' }), END])

  const bodyCut = plain.subarray(0, 512 + 512 + 300) // 두 번째 항목 본문 중간에서 끊김
  const brokenMid = Buffer.concat([tarFile('ok.txt', 3), Buffer.alloc(BLOCK, 0x41), END]) // 중간에 체크섬이 틀린 블록
  const noEnd = Buffer.concat([tarFile('a.txt', 10), tarFile('b.txt', 20)]) // 끝 표시 없음
  const partialBlock = Buffer.concat([tarFile('a.txt', 10), Buffer.alloc(100, 1)])
  const firstBroken = Buffer.alloc(BLOCK, 0x41)
  // base-256 크기, 음수 base-256, 숫자가 아닌 크기 필드
  const b256 = Buffer.alloc(12)
  b256[0] = 0x80
  b256.writeUIntBE(1500, 6, 6)
  const negative = Buffer.alloc(12)
  negative[0] = 0xc0
  const base256 = Buffer.concat([tarHeader({ name: 'big.bin', sizeField: b256, size: 0 }), pad(Buffer.alloc(1500)), tarFile('n.txt', 1), END])
  const negSize = Buffer.concat([tarHeader({ name: 'neg', sizeField: negative }), END])
  const badSize = Buffer.concat([tarHeader({ name: 'bad', sizeField: Buffer.from('12a45\0\0\0\0\0\0\0', 'latin1') }), END])

  const gz = (b: Buffer): Buffer => gzipSync(b)
  const gzPlain = gz(plain)
  // 멤버가 둘인 gzip: 앞 절반과 뒤 절반을 따로 압축해 이어 붙인다
  const mid = 512 * 3
  const gzMulti = Buffer.concat([gz(plain.subarray(0, mid)), gz(plain.subarray(mid))])

  return {
    'plain.tar': plain,
    'pax.tar': pax,
    'gnu-long.tar': gnuLong,
    'sparse.tar': sparse,
    'sparse-broken.tar': sparseBroken,
    'meta-loop.tar': metaLoop,
    'meta-huge.tar': metaHuge,
    'body-cut.tar': bodyCut,
    'broken-mid.tar': brokenMid,
    'no-end.tar': noEnd,
    'partial-block.tar': partialBlock,
    'first-broken.tar': firstBroken,
    'base256.tar': base256,
    'neg-size.tar': negSize,
    'bad-size.tar': badSize,
    'not-a.tar': Buffer.from('tar 가 아닌 글자들 '.repeat(100)),
    'empty.tar': Buffer.alloc(0),
    'plain.tar.gz': gzPlain,
    'plain.tgz': gzPlain,
    'multi-member.tar.gz': gzMulti,
    'truncated.tar.gz': gzPlain.subarray(0, gzPlain.length - 30),
    'pax.tar.gz': gz(pax),
    // gzip 이지만 tar 가 아님
    'single.txt.gz': gz(Buffer.from('그냥 파일 하나를 gzip 한 것\n'.repeat(30))),
    'bad.tar.gz': Buffer.concat([Buffer.from([0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3]), Buffer.from('garbage not deflate stream')])
  }
}

// ---------- 텍스트 ----------

function textFixtures(): Record<string, Buffer> {
  const ko = '한글 텍스트 abc\n가나다라마바사\n'
  const utf16 = (s: string, be: boolean): Buffer => {
    const le = Buffer.from(s, 'utf16le')
    if (be) le.swap16()
    return Buffer.concat([Buffer.from(be ? [0xfe, 0xff] : [0xff, 0xfe]), le])
  }
  const cp949 = Buffer.concat([CP949.hangul, Buffer.from(' '), CP949.test, Buffer.from('\nabc\n')])
  return {
    'utf8-bom.txt': Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(ko)]),
    'utf8.txt': Buffer.from(ko.repeat(3)),
    'utf16le.txt': utf16(ko, false),
    'utf16be.txt': utf16(ko, true),
    'cp949.txt': cp949,
    // CP949 이지만 UTF-8 로 우연히 유효해질 수 없는 긴 본문
    'cp949-long.txt': Buffer.concat(Array.from({ length: 40 }, () => cp949)),
    'multibyte.txt': Buffer.from('가나다라마'),
    // 이모지(4바이트)와 3바이트 문자가 섞임
    'emoji.txt': Buffer.from('a😀b가😀'),
    'empty.txt': Buffer.alloc(0),
    'ascii.txt': Buffer.from('line1\nline2\n'),
    // 앞부분은 UTF-8 이지만 마지막 바이트가 잘못됨(잘리지 않았다면 CP949 로 폴백)
    'bad-tail.txt': Buffer.concat([Buffer.from('abcdef 가나다'), Buffer.from([0xff])])
  }
}

// ---------- 케이스 ----------

export interface Case {
  file: string
  kind: 'text' | 'archive'
  /** text 는 maxBytes, archive 는 max */
  arg: number
}

export function buildFixtures(): Record<string, Buffer> {
  return { ...zipFixtures(), ...tarFixtures(), ...textFixtures() }
}

export function buildCases(files: string[]): Case[] {
  const cases: Case[] = []
  for (const file of files) {
    if (file.endsWith('.txt') && !file.endsWith('.txt.gz')) {
      // 멀티바이트가 걸치는 지점, 0, BOM 길이, 파일보다 큼
      for (const arg of [0, 1, 2, 3, 4, 5, 7, 8, 16, 1000]) cases.push({ file, kind: 'text', arg })
    } else {
      for (const arg of [0, 1, 2, 3, 100_000]) cases.push({ file, kind: 'archive', arg })
    }
  }
  return cases
}
