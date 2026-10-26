# 소리통 (Soritong)

한국어나 영어로 문장을 입력하면 Zoom, Google Meet, Microsoft Teams, Discord 등의 화상 회의에서 마이크 소리로 직접 출력해 주는 윈도우용 데스크톱 앱입니다.

![소리통 메인 화면](docs/screenshots/main-window.png)

---

## 소개

소리통은 직접 음성으로 말하기 어렵거나 목소리를 내지 않고 화상 회의나 음성 통화에 참여하고 싶은 사용자를 위한 도구입니다. 성대 피로, 조용한 사무실 환경, 목소리 회복 기간, 또는 음성 보조가 필요한 상황에서도 키보드로 입력한 내용을 자연스러운 음성으로 전달할 수 있습니다.

- **마이크 직접 송출**: 가상 마이크 케이블을 통해 입력 음성이 회의 참여자에게 바로 전달되며, 주변 배경 소음이 섞이지 않습니다.
- **동시 모니터링**: 이어폰이나 헤드폰을 통해 내가 입력한 문장이 어떻게 발음되는지 실시간으로 함께 들을 수 있습니다.
- **한국어 및 영어 지원**: 한국어와 영어 문장을 자연스럽게 읽어줍니다.
- **오프라인 및 클라우드 음성 엔진**: 인터넷 연결 없이 그래픽카드(GPU)에서 완전히 비공개로 동작하는 오프라인 모델부터 고품질 클라우드 음성까지 선택하여 사용할 수 있습니다.

---

## 동작 원리

```
+-----------------------------------------------------------+
|                          소리통                           |
|    [ 한국어 / 영어 문장 입력 ]  --->  [ TTS 음성 엔진 ]   |
+-----------------------------------------------------------+
                               |
                               v
                     [ VB-CABLE Input ]
                               |
              +----------------+----------------+
              |                                 |
              v                                 v
        [ 화상 회의 앱 ]                  [ 이어폰/헤드폰 ]
     Zoom / Meet / Discord                로컬 모니터링
 (마이크: CABLE Output 선택)            (내가 보낸 음성 확인)
```

1. 소리통 입력창에 문장을 적고 **Enter** 키를 누릅니다.
2. 선택한 음성 엔진이 문장을 음성으로 변환하여 가상 오디오 케이블(VB-CABLE)로 전송합니다.
3. Zoom이나 Google Meet의 마이크 장치를 **CABLE Output (VB-Audio Virtual Cable)**으로 설정합니다.
4. 회의 상대방에게는 선명한 음성으로 전달됩니다.

---

## 설정 마법사

소리통을 처음 실행하면 내장된 설정 마법사가 단계별로 오디오 설정을 도와줍니다:

![소리통 설정 마법사](docs/screenshots/wizard-step-1.png)

마법사가 제공하는 기능:
- 클릭 한 번으로 공식 VB-CABLE 설치 파일을 내려받아 실행합니다.
- 기존 기본 스피커와 마이크 설정을 안전하게 보존하고 복원합니다.
- 실제 회의에 들어가기 전 오디오 루프백 테스트를 통해 소리가 마이크로 정상 전달되는지 점검합니다.
- Zoom 및 화상 회의 프로그램에서 마이크를 설정하는 방법을 안내합니다.

설정 마법사는 앱 설정 메뉴에서 언제든지 다시 열 수 있습니다.

---

## 지원 음성 엔진

소리통은 5가지 음성 합성 엔진을 지원합니다:

| 엔진 | 유형 | 특징 및 요구 사항 |
| :--- | :--- | :--- |
| **Supertonic 3** | 로컬 오프라인 | 사용자의 PC GPU(DirectML)에서 직접 동작합니다. 인터넷 연결이나 API 키가 필요 없으며 데이터가 외부로 전송되지 않습니다. |
| **Edge Online** | 무료 클라우드 | Microsoft Edge의 자연스러운 한국어/영어 음성을 사용합니다. 별도의 API 키 없이 무료로 빠르고 안정적인 음성을 제공합니다. |
| **ElevenLabs** | 유료 클라우드 | 사용자 본인의 음성 복제(Voice Clone)와 최고 품질의 신경망 음성을 지원합니다. 개인 ElevenLabs API 키가 필요합니다. |
| **OpenAI** | 유료 클라우드 | OpenAI의 고품질 음성 모델(Alloy, Echo, Nova 등)을 지원합니다. 개인 OpenAI API 키가 필요합니다. |
| **Windows System** | 로컬 오프라인 | 윈도우에 기본 탑재된 SAPI 음성을 사용합니다. 추가 다운로드 없이 즉시 동작합니다. |

클라우드 엔진에 사용되는 API 키는 Windows 자격 증명 관리자(Credential Manager)에 안전하게 암호화되어 저장됩니다.

---

## 설치 및 다운로드

GitHub Releases 페이지에서 최신 버전을 내려받으실 수 있습니다:

- **설치 프로그램 (`soritong-x.x.x-windows-x64-setup.exe`)**:
  권장 옵션입니다. 관리자 권한 없이 현재 사용자 계정에 안전하게 설치되며, 시작 메뉴 바로가기와 윈도우 설정의 앱 설치/제거 기능을 지원합니다.
- **포터블 압축 파일 (`soritong-x.x.x-windows-x64-portable.zip`)**:
  설치 과정 없이 원하는 폴더에 압축을 풀고 `soritong-app.exe`를 바로 실행할 수 있습니다. 필요한 모든 런타임 라이브러리(`DirectML.dll` 및 Microsoft Visual C++ CRT)가 포함되어 있습니다.

### 시스템 요구 사항

- Windows 10 (버전 1809 이상) 또는 Windows 11 (64비트 x64).
- Microsoft Edge WebView2 런타임 (최신 윈도우에 기본 설치되어 있습니다).
- DirectX 12 지원 그래픽카드 (Supertonic 3 오프라인 GPU 가속 사용 시).

---

## 단축키

- **Enter**: 현재 입력한 문장을 즉시 음성으로 송출합니다.
- **Shift + Enter**: 줄 바꿈을 입력하여 여러 문장을 작성합니다.
- **Ctrl + Alt + Space**: 다른 작업을 하는 중에도 소리통 창을 화면 맨 앞으로 불러옵니다 (단축키는 변경 가능).

---

## 회의 프로그램 연동 방법

### Zoom
1. Zoom **설정** > **오디오**로 이동합니다.
2. **마이크** 장치를 `CABLE Output (VB-Audio Virtual Cable)`으로 선택합니다.
3. **스피커** 장치는 평소 사용하시는 이어폰이나 헤드폰으로 설정합니다.
4. "마이크 볼륨 자동 조정" 체크를 해제하시는 것을 권장합니다.

### Google Meet
1. Google Meet 화면 하단의 **옵션 더보기** (...) > **설정** > **오디오**를 클릭합니다.
2. **마이크** 장치를 `CABLE Output (VB-Audio Virtual Cable)`으로 선택합니다.
3. **스피커** 장치는 평소 사용하시는 이어폰이나 헤드폰으로 설정합니다.

### Discord
1. **사용자 설정** > **음성 및 비디오**로 이동합니다.
2. **녹음 장치(Input Device)**를 `CABLE Output (VB-Audio Virtual Cable)`으로 선택합니다.
3. **출력 장치(Output Device)**를 평소 사용하시는 헤드폰으로 선택합니다.

---

## 소스 코드 빌드 (개발자 안내)

```powershell
# 사전 준비: Rust 2024 툴체인, Node.js >= 22
git clone https://github.com/shyoo/soritong.git
cd soritong

# 단위 및 통합 테스트 실행
cargo test --workspace

# UI 자동화 테스트 실행
node --test crates/soritong-app/ui-tests/keys.test.mjs crates/soritong-app/ui-tests/wizard.test.mjs

# 릴리스 패키징 (NSIS 설치 프로그램 및 포터블 ZIP 생성)
pwsh -ExecutionPolicy Bypass -File scripts/release.ps1
```

---

## 라이선스 및 타사 고지사항

소리통은 [Apache-2.0 License](LICENSE)에 따라 배포되는 오픈소스 소프트웨어입니다.

Supertonic 3 (BigScience OpenRAIL-M), VB-CABLE, ONNX Runtime, DirectML 및 Microsoft Visual C++ 런타임에 대한 상세 라이선스 정보는 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)를 참고해 주십시오.
