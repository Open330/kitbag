<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.png">
  <img src="assets/logo.png" width="84" alt="kitbag">
</picture>

# kitbag

**이 기계가 들고 있는 당신의 것 전부** — 무엇이고, 누구 것이며,
다음 기계로 어떻게 옮기는가.

[![ci](https://github.com/Open330/kitbag/actions/workflows/ci.yml/badge.svg)](https://github.com/Open330/kitbag/actions/workflows/ci.yml)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)
[![status](https://img.shields.io/badge/status-design-lightgrey.svg)](DESIGN.md)

[English](README.md) · 한국어

<img src="docs/demo.gif" width="860" alt="kitbag status">

</div>

> **상태: 초기지만 동작합니다.** 아래 명령은 전부 실제 기계와 실제 저장소(`bw`·
> `op`·`pass`·`age` 암호화 파일)를 상대로 제 일을 합니다. 남은 건 설치기 쪽입니다 —
> 레시피가 패키지·링크·macOS 설정·명령까지 커버하고, 기계 설정의 나머지는 앞으로
> 입니다. 근거와 계획은 [DESIGN.md](DESIGN.md)에 있습니다.

## 여기서 시작하세요

```bash
kitbag backup
```

한 번도 해본 적 없는 기계에서 상태가 스토어에 들어간 기계까지, 명령 하나로
갑니다. 어느 스토어인지, 이 기계가 무엇을 맡을지 묻고, 그다음 찾은 것들을
**하나씩** 묻습니다 — 자기 홈 디렉터리에 대해 "이거 전부 가져가"는 보지 않고
할 수 있는 대답이 아니니까요:

```console
  3/4  3 thing(s) here that nothing keeps.
       [y]es  [n]ot now  [d]ismiss for good  [a]ll  [q]uit asking

       ~/.ssh/id_*
       a private key, and this machine's own · a known place
       [y/n/d/a/q] y

  4/4  What would be sent:

  + ssh:id_ed25519@this-mac       would be sent
  + programs@this-mac             would be sent

  Send these? [y/N]
```

마지막 질문 전까지 아무것도 보내지 않고, 그 위의 줄이 계획입니다. 각 단계는
같은 이름의 명령 그대로입니다 — `discover`, `track`, `push`, `resolve` —
그래서 이 워크스루는 **실행 순서**지, 따로 관리해야 할 두 번째 구현이
아닙니다.

## 아무도 답하지 않는 질문

새 기계를 세팅하는 일은 두 종류의 도구가 나눠 맡고 있습니다. dotfile 관리자는
설정을 옮기고, 비밀번호 관리자는 비밀을 옮깁니다. 하지만 기계가 둘 이상이고
소속이 둘 이상이 되는 순간 진짜 중요한 질문에는 어느 쪽도 답하지 못합니다.

> **이 기계에서 회사 것은 무엇이고, 내가 퇴사하면 그것들은 어떻게 되는가?**

kitbag은 패키지·설정·시스템 설정·자격증명·앱 데이터를 **같은 종류**로 다룹니다 —
원하는 상태, 출처, 적용 방법, 그리고 **소유자**. 그리고 모든 것을 그 소유자
기준으로 거릅니다.

| scope | 뜻 |
| :-- | :-- |
| `personal` | 내 것 |
| `work` | 회사나 고객의 것 |
| `shared` | 남이 소유하고 나에게 접근을 준 계정 |
| `mixed` | 하나에 여러 삶이 섞인 것 — 무엇과 무엇인지 밝혀야 함 |
| `local` | 이 기계 전용, 어디로도 안 나감 |

기계는 자기가 받을 scope를 선언합니다. 개인 노트북은 회사 자격증명을 복원하지
않고, 회사 기계는 개인 취미 도구를 설치하지 않습니다. 같은 필터가 **무엇을
보낼지, 무엇을 쓸지, 보고에 무엇을 보여줄지**를 전부 결정합니다.

## 설치

```bash
curl -LsSf https://raw.githubusercontent.com/Open330/kitbag/main/install.sh | sh
```

플랫폼을 감지하고, 릴리스와 함께 게시된 체크섬으로 다운로드를 검증한 뒤,
바이너리 하나를 `~/.local/bin`에 둡니다. 또는 `cargo install --path crates/kitbag-cli`.

## 명령

```console
kitbag status                 이 기계에 무엇이 있고, 저장소와 무엇이 다른지
kitbag plan                   apply가 무엇을 바꿀지만 보여주고 멈춤
kitbag apply                  기계를 레시피대로 맞춤
                              (패키지·링크·defaults·다운로드·클론·병합)
kitbag discover               추적되지 않는 상태, 그리고 추적은 되는데
                              저장소에 없는 것
kitbag track <path> --scope work
kitbag programs               무엇이 깔려 있는지 적어둠 — 다시 깔 수 있도록
kitbag push / restore         scope 단위로 옮김
kitbag diff                   무엇이 다른지, 값은 빼고
kitbag resolve                어느 쪽도 혼자 정할 수 없는 것을 정함
kitbag doctor                 권한, 도달성, 미분류 파일, 저장소의 고아 항목
kitbag lint                   커밋되면 안 되는 것을 거부
kitbag trust sync             이 기계에 로그인할 수 있는 기계들
kitbag completions zsh        …bash, fish, elvish, powershell
```

모든 명령에 `--json`, `--color auto|always|never`, `NO_COLOR` 존중. 그리고
**어떤 명령도 비밀의 값을 출력하지 않습니다.**

## 두 기계가 어긋났을 때

차이에는 방향이 있습니다. kitbag이 **마지막으로 합의한 시점의 지문**을
기록하기 때문입니다 — git이 merge base라 부르는 그 세 번째 점입니다.

```console
>  내 쪽만 움직임        push가 보냄
<  저장소만 움직임       restore가 가져옴
!  양쪽 다 움직임        사람이 정할 일
```

push는 `<`를 보내지 않고 restore는 `>`를 가져오지 않습니다. 둘 다 **더
새것 위에 옛것을 쓰는** 동작입니다. `!`는 양쪽을 멈추고 기다립니다:

```console
$ kitbag resolve
! env:docs-publish  (1/2)
    here   3 lines
    store  5 lines
    only there:  DOCS_ROOT DOCS_USER
    differ:      DOCS_URL
    [m]ine  [t]heirs  [s]kip  [q]uit >
```

키 이름·개수·크기, 그리고 아카이브 안의 파일 목록 — **양쪽 어느 값도
나오지 않습니다.** 못 알아들은 답은 skip이고 빈 줄도 skip입니다. 실제 답
둘 중 하나는 자격증명을 덮어쓰므로, 실수로 가장 누르기 쉬운 키가 아무것도
하지 않아야 합니다. 터미널이 없으면 묻지 않고 남은 것만 나열합니다.

## 한 기계에만 속하는 것

네 기계가 한 경로에서 같은 이름을 유도하고 그 아래 서로 다른 것을 들고
있을 수 있습니다. SSH 키가 그렇습니다 — 두 기계가 한 키를 쓰면 그 키를
폐기할 때 둘 다 잠깁니다.

```toml
[[track]]
path = "~/.ssh/id_ed25519"
per_machine = true          # ssh:id_ed25519@<host>
```

네 항목, 네 키, 각각 보관됩니다. 그리고 **파일은 `~/.ssh/id_ed25519`에
그대로 있습니다** — ssh가 찾는 자리입니다. 저장소에서의 이름만 다르고,
복구는 다른 기계 이름이 찍힌 것을 건드리지 않습니다.

`skip`은 다른 답입니다. **어느 방향으로도 상대하고 싶지 않은 항목**을 위한
것이지, 단지 이 기계 것인 항목을 위한 게 아닙니다. 키를 주고받지 않겠다는
건 그 키를 백업하지 않겠다는 뜻이고, 한 곳에만 있는 키는 그 기계와 함께
사라집니다.

## 프로그램은 목록으로

저장소가 절대 담지 말아야 할 것이 바이너리입니다. 크고, 한 아키텍처용으로
빌드됐고, 그걸 배포한 쪽이 다시 내어줍니다. 보관할 값어치가 있는 건
**무엇이 깔렸는가**입니다:

```console
$ kitbag programs
kitbag/programs 1
brew	ripgrep
cask	ghostty
cargo	kitbag	0.13.0
npm	@bitwarden/cli	2026.8.0
rustup	stable-aarch64-apple-darwin
```

71개가 1킬로바이트입니다. `kitbag programs --restore`로 되돌리면 **빠진
것만 설치하고 아무것도 지우지 않습니다.** 기계가 목록보다 많이 가진 건
괜찮습니다. 목록은 *없어서는 안 되는 것*입니다. 다룰 줄 모르는 관리자는
추측하지 않고 그렇다고 말합니다.

필요한 게 애초에 관리자에 없었다면 얘기가 달라집니다. `rustup`, `uv`,
`nvm`, 그리고 남의 설치 스크립트마다 들어있는 `curl … | sh` — 어떤 패키지
목록도 설명해주지 않는 기계의 5분의 1입니다. 툴체인이 빠진 목록으로는
기계를 다시 세울 수 없으니, 기계가 직접 선언할 수 있습니다:

```toml
[[program]]
name = "uv"
install = "curl -LsSf https://astral.sh/uv/install.sh | sh"
version_from = "uv --version"     # 선택; 출력에서 버전을 읽습니다
present = "uv --version"          # 선택; 기본값은 `command -v uv`
```

**실제로 있을 때만** 기록됩니다 — 아무도 실행하지 않은 선언은 사실이 아니라
계획입니다. 설치 줄은 항목과 함께 이동하므로, 다시 세워지는 기계는 아직
가지고 있지도 않은 설정 파일이 아니라 저장소에서 설치법을 배웁니다. kitbag은
그 줄을 실행하기 전에 먼저 보여줍니다. 저장소에서 나온 명령은 여전히
저장소에서 나온 명령이니까요.

다른 기계의 목록도 읽을 수 있습니다. 베끼고 싶은 기계가 죽은 그 기계일 때
정확히 필요한 기능입니다:

```console
$ kitbag programs --list
jiun-mbp                     programs@jiun-mbp
june-mbp                     programs@june-mbp

$ kitbag programs --from jiun-mbp              # 읽기
$ kitbag programs --from jiun-mbp --restore    # 또는 그 기계가 되기
```

추적 항목으로는 다른 명령 쌍과 똑같습니다:

```toml
[[track]]
name = "programs"
scope = "personal"
per_machine = true
command = { export = "kitbag programs", restore = "kitbag programs --restore" }
```

`kitbag restore`가 쓰기 전에 물어보는 이유이기도 합니다. 파일을 되돌리는 건
되돌릴 수 있습니다 — 각각 먼저 백업하니까요. 소프트웨어 설치는 아닙니다.
그래서 명령 전체가 한 번 멈추고 묻습니다. `-y`로 미리 답할 수 있고,
`--dry-run`은 무슨 일이 일어날지 정확히 말하면서 아무것도 쓰지 않습니다.

## 세 개의 저장소, 그리고 그 이유

공개 dotfiles 레포가 안전한 조건은 "비밀을 넣지 않는 것"이 아닙니다. **무엇이
존재하는지의 목록을 빼는 것**입니다. `~/.envs/kibana.env → work` 한 줄이
고용주와 스택과 표적을 알려주고, 여기엔 비밀이 전혀 필요 없습니다.

```
  repo (공개)         레시피와 프로바이더 — 고용주도 서비스 이름도 없음
  inventory (비공개)  무엇이 있고 어디로 가고 어느 scope인지 — 저장소 안에
  secret store        값 자체 — 볼트, 또는 암호화된 파일
```

수집은 **패턴으로만**(`~/.envs/*.env`), 이름으로는 절대 하지 않습니다. scope는
파일이 들고 다니는 마커(`# scope: work`)이지 레포 안의 표가 아닙니다.
`kitbag lint`가 이 둘을 어기는 커밋을 거부하고, **이 레포 자신에게도 매 CI마다
적용**됩니다 — 자기 저자의 기계를 흘리면서 만들어진 도구는 자기 설계를 반박하는
셈이니까요.

## 비밀 저장소

kitbag은 저장소를 직접 만들지 않습니다. 쓰시던 것을 빌려 씁니다 — kitbag에
흥미를 잃더라도 비밀이 그 안에 갇히지 않도록.

| 백엔드 | 저장소 | |
| :-- | :-- | :-- |
| `bw` | Bitwarden / Vaultwarden — 무료 티어, 셀프호스팅 가능 | v0.1 |
| `op` | 1Password — 이 분야 최고의 개발자 CLI | v0.1 |
| `pass` | `pass` / `gopass` — GPG와 git 저장소 | v0.2 |
| `age` | `age`로 암호화한 파일 — 서버가 아예 필요 없음 | v0.2 |

백엔드 추가는 `list`/`get`/`put` 어댑터가 전부입니다. kitbag이 항목에 대해 알아야
하는 것은 전부 **자기 봉투 안에** 들어가므로, 이름으로 바이트를 보관할 수 있는
저장소면 충분합니다.

```text
kitbag/1
scope: work
owner: acme
encoding: utf8
sha256: 1f0e3d…

export TOKEN=…
```

<details>
<summary>백엔드 트레이트에 <code>delete</code>가 없는 이유</summary>

저장소에는 이 기계가 모르는 것들이 들어 있습니다 — 다른 기계의 키, 누군가
추가한 계정. 모르는 것을 지우는 도구는 언젠가 중요한 것을 지웁니다. 삭제는
저장소 본래의 클라이언트로, 사람이 내리는 결정입니다.

</details>

## 비슷한 도구들

[chezmoi](https://www.chezmoi.io/)와 [yadm](https://yadm.io/)은 dotfiles를 잘
관리하지만 소유권을 모델링하지 않고 자격증명을 옮기지 않습니다.
[1Password CLI](https://developer.1password.com/docs/cli/)와
[SOPS](https://github.com/getsops/sops)는 비밀을 다루지 기계를 다루지 않습니다.
[Mackup](https://github.com/lra/mackup)은 앱 상태를 옮겼지만 관리가 중단됐습니다.
kitbag은 그 교집합입니다 — **어디에 있든 개인 상태를, 소유자를 붙여서.**

## 빌드

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

기여는 환영합니다. 먼저 [CONTRIBUTING.md](CONTRIBUTING.md)를 읽어 주세요 —
첫 번째 규칙은 **누구의 기계도 이 레포에 들어가지 않는다**입니다.

<div align="center"><sub>MIT · <a href="DESIGN.md">DESIGN.md</a></sub></div>
