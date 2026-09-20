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

> **상태: 초기.** `status`와 `plan`은 실제 기계를 읽고 진짜로 답합니다.
> `apply`·`push`·`restore`·`discover`는 아직 스텁입니다 — 근거와 계획은
> [DESIGN.md](DESIGN.md)에 있습니다.

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

## 명령

```console
kitbag status                 이 기계에 무엇이 있고, 저장소와 무엇이 다른지
kitbag plan                   apply가 무엇을 바꿀지만 보여주고 멈춤
kitbag apply                  기계를 레시피대로 맞춤
kitbag discover               아직 아무도 추적하지 않는 개인 상태를 찾아냄
kitbag track <path> --scope work
kitbag push / restore         scope 단위로 옮김
kitbag doctor                 권한, 도달성, 미분류 파일, 저장소의 고아 항목
kitbag lint                   커밋되면 안 되는 것을 거부
kitbag completions zsh        …bash, fish, elvish, powershell
```

모든 명령에 `--json`, `--color auto|always|never`, `NO_COLOR` 존중. 그리고
**어떤 명령도 비밀의 값을 출력하지 않습니다.**

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
