# Third-Party Notices and Licenses

Soritong (소리통) incorporates or interacts with third-party software components and models.
This document describes the third-party components, their respective authors, and their licensing terms.

---

## 1. Supertonic 3 (Text-to-Speech Model)

- **Author / Provider**: Supertone Inc. / HYBE
- **Repository / Weights**: Hugging Face (`Supertone/supertonic-3`)
- **License**: BigScience OpenRAIL-M License
- **Notice**:
  Supertonic 3 model weights are downloaded at runtime upon user request and cached locally.
  Use of the Supertonic 3 model weights is governed by the BigScience OpenRAIL-M License
  (https://huggingface.co/spaces/bigscience/license).
  Under this license, the model may not be used for prohibited use cases including illegal acts,
  generating non-consensual biometric impersonation, discrimination, or intentional harm.

---

## 2. VB-CABLE Virtual Audio Device

- **Author / Provider**: Vincent Burel / VB-AUDIO Software (https://vb-audio.com/Cable/)
- **License**: Donationware for Personal End-User Use; Commercial Licensing required for corporate use
- **Notice**:
  VB-CABLE is developed and copyrighted by VB-AUDIO Software.
  Soritong does not bundle or redistribute the VB-CABLE driver installer inside its application binary.
  Instead, Soritong provides setup automation that downloads the official driver archive directly from
  VB-Audio's official distribution servers upon explicit user consent.
  Personal use of VB-CABLE is donationware. Commercial or enterprise environments require a commercial
  license directly from VB-AUDIO Software.

---

## 3. Microsoft Edge Online Voices (Read Aloud)

- **Provider**: Microsoft Corporation
- **Notice**:
  The Edge online speech engine connects to the speech synthesis services used by Microsoft Edge.
  Use of these online voices is subject to Microsoft's Terms of Use and Privacy Statement.
  This service requires an active Internet connection and does not run locally on your device.

---

## 4. ONNX Runtime

- **Copyright**: Microsoft Corporation
- **License**: MIT License
- **License Text**:
  ```
  Copyright (c) Microsoft Corporation. All rights reserved.

  Permission is hereby granted, free of charge, to any person obtaining a copy
  of this software and associated documentation files (the "Software"), to deal
  in the Software without restriction, including without limitation the rights
  to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
  copies of the Software, and to permit persons to whom the Software is
  furnished to do so, subject to the following conditions:

  The above copyright notice and this permission notice shall be included in all
  copies or substantial portions of the Software.

  THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
  IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
  FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
  AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
  LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
  OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
  SOFTWARE.
  ```

---

## 5. DirectML (Direct Machine Learning)

- **Copyright**: Microsoft Corporation
- **License**: MIT License / Microsoft DirectML Redistributable License
- **Notice**:
  `DirectML.dll` provides hardware-accelerated machine learning execution on Windows DirectX 12 compatible GPUs.
  Redistributed in binary form in accordance with the Microsoft DirectML redistributable licensing terms.

---

## 6. Microsoft Visual C++ Runtime (MSVC CRT)

- **Copyright**: Microsoft Corporation
- **License**: Microsoft Software License Terms - Microsoft Visual Studio 2022 (Redistributable Code)
- **Files**: `msvcp140.dll`, `msvcp140_1.dll`, `msvcp140_2.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`, etc.
- **Notice**:
  Redistributed in binary form alongside application binaries in accordance with the Microsoft Visual Studio
  redistribution terms to enable running without requiring a separate Visual C++ Redistributable package installation.

---

## 7. Tauri

- **Copyright**: Tauri Programme within The Commons Conservancy
- **License**: Apache License, Version 2.0 or MIT License (Dual Licensed)
- **Notice**:
  Licensed under either Apache License 2.0 (https://www.apache.org/licenses/LICENSE-2.0) or MIT License
  (https://opensource.org/licenses/MIT).

---

## 8. Microsoft Edge WebView2

- **Provider**: Microsoft Corporation
- **License**: Microsoft Software License Terms - Microsoft Edge WebView2 Runtime
- **Notice**:
  Soritong uses the Microsoft Edge WebView2 runtime to render its user interface. The WebView2 Runtime
  is typically pre-installed on Windows 10 and Windows 11.
