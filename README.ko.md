# 소리통 (soritong)

한국어나 영어로 입력하고 Enter를 누르면, 소리통이 Zoom, Google Meet, Teams, Discord에서 내 마이크로
말해 줍니다. 소리 내어 말하지 않고도 통화에 참여할 수 있습니다.

![소리통 메인 화면](docs/screenshots/main-window.png)

매달 Zoom으로 하는 한국 독서 모임에서 시작됐습니다. 한 회원에게는 모임 시간이 새벽 4~5시라, 가족이
자는 동안 소리를 낼 수 없었습니다. 채팅 글보다 목소리가 다른 회원들에게 더 가깝게 느껴집니다.

[English README](README.md)

## 다운로드

[Releases](https://github.com/shyoo/soritong/releases)에서 최신 버전을 받으세요 (Windows 10/11, x64).

- **`soritong-<버전>-windows-x64-setup.exe`**: 설치 프로그램. 현재 사용자에게만 설치하므로 관리자
  권한이 필요 없고, 시작 메뉴 항목과 제거 프로그램을 추가합니다.
- **`soritong-<버전>-windows-x64-portable.zip`**: 아무 폴더에나 압축을 풀고 `soritong.exe`를 실행합니다.

`SHA256SUMS.txt`에 두 파일의 체크섬이 있습니다. 아직 코드 서명이 없어서 처음 실행할 때 Windows
SmartScreen 경고가 뜰 수 있습니다. **추가 정보**를 누른 뒤 **실행**을 누르세요.

## 동작 방식

Zoom은 녹음 장치에서만 소리를 받습니다. Windows 앱은 드라이버 없이 녹음 장치를 만들 수 없어서, 소리통은
무료 가상 케이블 [VB-CABLE](https://vb-audio.com/Cable/)을 씁니다. 소리통이 케이블 입력에 음성을
재생하면, Zoom은 케이블 출력을 내 마이크로 씁니다.

```
 입력 ──> 소리통 ──> TTS 음성 ──┬──> CABLE Input ══> CABLE Output ──> Zoom (마이크)
                               └──> 내 헤드폰 (선택, 내 목소리 확인용)
```

케이블에는 소리통만 쓰기 때문에, 회의 참가자의 목소리가 다시 회의로 되돌아가는 일은 없습니다.

### 설정 마법사

처음 실행하면 마법사가 설정을 대신 해 줍니다.

![설정 마법사](docs/screenshots/wizard.png)

1. 공식 VB-CABLE 설치 파일을 내려받아(체크섬 고정) 실행합니다. 드라이버 설치에 관리자 권한을 묻습니다.
2. VB-CABLE이 기본 스피커와 마이크를 가져가 버리는 경우가 있어, 원래 쓰던 장치로 되돌립니다.
3. 소리통의 출력을 `CABLE Input`으로 맞춥니다.
4. 테스트 음을 케이블로 보내, 통화 전에 소리가 실제로 전달되는지 확인합니다.
5. Zoom에서 무엇을 고르면 되는지 보여주고, 마이크 이름을 복사하는 버튼을 줍니다.

마법사는 설정에서 언제든 다시 열 수 있습니다.

**설치 없이 쓰기:** 마법사에서 "추가 설치 없이 사용"를 고르면 소리통이 일반 스피커로 재생합니다.
Zoom에서는 **화면 공유 > 고급 > 컴퓨터 오디오**로 소리를 공유합니다. 드라이버가 필요 없지만, PC의
다른 소리도 함께 공유되고 회의마다 공유를 시작해야 합니다.

## 음성

| 엔진 | 실행 위치 | 설명 |
| :--- | :--- | :--- |
| **Supertonic 3** | 이 PC | 무료이고, 입력한 글이 PC 밖으로 나가지 않습니다. 한국어와 영어. 처음 쓸 때 약 400 MB 모델을 내려받습니다. CPU에서 동작합니다. |
| **Microsoft Edge 음성** | 온라인 | 무료, 키 불필요, 자연스러운 한국어·영어. Edge의 "소리 내어 읽기"와 같은 비공식 서비스라 막히거나 느려질 수 있습니다. |
| **ElevenLabs** | 온라인 | 내 [목소리 복제](https://elevenlabs.io/voice-cloning)나 ElevenLabs 음성. API 키가 필요하고 크레딧을 씁니다. 다시 재생은 로컬 캐시에서 나와 크레딧이 들지 않습니다. |
| **OpenAI 호환** | 온라인 또는 로컬 | OpenAI `/v1/audio/speech`, 또는 같은 API를 제공하는 로컬 서버(예: 내 GPU에서 돌리는 목소리 복제 모델). |
| **Windows 기본 음성** | 이 PC | 설정이 필요 없지만 기계음에 가깝습니다. 한국어는 Windows 설정에서 한국어 언어와 음성 합성 기능을 설치해야 합니다. |

API 키는 파일이 아니라 Windows 자격 증명 관리자에 저장됩니다.

## 사용법

- **Enter**: 말하기. **Shift+Enter**: 줄 바꿈. **Esc**: 멈추기. **위쪽 화살표**: 마지막 메시지 다시 불러오기.
- **Ctrl+Alt+Space**: Zoom을 보다가도 소리통을 앞으로 가져옵니다.
- 빠른 문구(네, 맞아요 / 잠시만요 / ...)를 누르면 바로 말합니다. 연필 버튼으로 문구를 고칠 수 있습니다.
- 메시지 옆 재생 버튼으로 다시 말합니다.
- 헤드폰 버튼을 켜면 회의에 나가는 소리를 내 헤드폰으로도 들을 수 있습니다.

### Zoom 설정

1. **설정 > 오디오 > 마이크**: `CABLE Output (VB-Audio Virtual Cable)`.
2. **스피커**는 평소 쓰는 헤드폰 그대로 둡니다.
3. **마이크 볼륨 자동 조정**을 끕니다. 음성이 끊기면 **배경 소음 억제**를 **낮음**으로 바꿉니다.

Google Meet, Teams, Discord도 같습니다. 마이크로 `CABLE Output (VB-Audio Virtual Cable)`을 고르세요.

## 저장 위치

- 설정: `%APPDATA%\soritong\settings.json`
- 음성 모델과 음성 캐시: `%LOCALAPPDATA%\soritong`

제거해도 두 폴더는 남아서, 다시 설치하면 설정이 그대로입니다. 완전히 지우려면 두 폴더를 삭제하세요.
VB-CABLE은 자체 설치 프로그램으로 따로 제거합니다.

## 소스에서 빌드

Rust(stable, 1.85 이상), Node.js 22, Visual Studio C++ 빌드 도구가 필요합니다.

```powershell
git clone https://github.com/shyoo/soritong.git
cd soritong
cargo test --workspace
cargo run -p soritong-app                 # 앱
cargo run -p soritong-cli -- --help       # 엔진 시험용 명령줄 도구
pwsh -File scripts/release.ps1            # dist/에 설치 파일과 포터블 ZIP (tauri-cli 필요)
```

코드 구조와 확인 절차는 [AGENTS.md](AGENTS.md), 설계와 검토한 대안은 [docs/PLAN.md](docs/PLAN.md)에
있습니다.

## 라이선스

Apache-2.0, [LICENSE](LICENSE) 참고. Supertonic 3 모델 가중치는 이 저장소에 포함되지 않으며,
Supertone에서 BigScience OpenRAIL-M 라이선스로 내려받습니다(유해한 용도 제한). VB-CABLE은 VB-Audio의
도네이션웨어로, 번들하지 않고 VB-Audio에서 직접 내려받습니다. [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) 참고.
