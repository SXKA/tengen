# Tengen

[![CI](https://github.com/SXKA/tengen/actions/workflows/ci.yml/badge.svg)](https://github.com/SXKA/tengen/actions/workflows/ci.yml)
[![License: GPL v3](https://img.shields.io/badge/授權-GPLv3-blue.svg)](LICENSE)
[![English](https://img.shields.io/badge/lang-English-red.svg)](README.md)

Tengen 是用 Rust 寫的五子棋與連珠引擎。它可以編譯成 WebAssembly 在瀏覽器裡下，也能以
`pbrain-tengen` 的形式在 Piskvork 等 [Gomocup](https://gomocup.org/) 對戰平台上使用。

天元是棋盤正中央的點，多數對局從這裡開始。

## 開始下棋

- **瀏覽器**：打開儲存庫 About 欄位裡的網站連結。引擎在你的瀏覽器（Web Worker）裡運算，不會把棋局送到伺服器。
- **Piskvork 等 Gomocup 平台**：到 [Releases](https://github.com/SXKA/tengen/releases/latest) 下載對應系統的
  `pbrain-tengen`，加入成 brain 即可。支援 5×5 到 20×20 的棋盤，以及 Piskvork 協定的所有規則。

## 規則

| `INFO rule` | 規則 | 勝利條件 |
| --- | --- | --- |
| 0 | 自由 | 五子或以上連線 |
| 1 | 標準 | 剛好五子；長連不算 |
| 4 | 連珠 | 黑棋剛好五子，且不能下長連、四四、三三（禁手）；白棋五子以上 |
| 8 | 自由 Caro | 六子以上，或兩端沒有都被棋子擋住的五子 |
| 9 | Caro（Gomocup） | 剛好五子，且兩端沒有都被棋子擋住；棋盤邊緣不算擋 |

規則數字再加 2（連續對局）也支援：裁判標成不屬於任何一方的子（`BOARD` 的第 3 種）會同時擋住雙方。brain 也會回應 swap2
開局的 `SWAP2BOARD`。

勝利條件和連珠禁手都在 CI 裡用數千個隨機局面與 [Gomocup 官方裁判程式](https://github.com/Gomocup/GomocupJudge) 逐點比對。

## 運作方式

設計參考頂尖的傳統五子棋引擎，特別是 [Rapfi](https://github.com/dhbloo/rapfi)，但不使用神經網路：

- **位元棋盤**：每種顏色、每個方向、每一條線各用一個位元遮罩表示，任一格周圍的窗口只需一次位移。
- **棋型表**：2¹⁸ 項的查表把每個 9 格窗口歸類成 13 種單線棋型（活三、衝四等）。其他規則要多看一格，才分得出五連與長連、
  以及 Caro 裡的棋子和棋盤邊緣，所以每種規則、每種顏色各有一張 2²⁰ 項的表。每個空格快取雙方四個方向的棋型、與順序無關的
  組合編碼，以及組合類別（四三、雙三等）。落子時只更新那四條線上的格子。
- **連珠**：長連和四四直接從表裡讀出。三三要兩個三都能用「本身不是禁手」的一手變成活四才算禁手，這點用實際試下、遞迴檢查。
  黑棋絕不下禁手；白棋的衝四若只能擋在禁手上，白棋就贏。
- **線性估值**：每個空格依組合編碼提供分數，再加上「雙方各有哪些先手」的威脅項。表分成開局與後盤兩張，依盤上子數漸變
  （tapered evaluation）。權重用自我對弈的勝負做邏輯迴歸擬合（Texel tuning），是量出來的而不是猜的。
- **學出來的著法排序**：依「這手建立了什麼、擋掉了什麼」的組合編碼給分，分數擬合自我對弈實際下的著法。
- **搜尋**：迭代加深的主要變例搜尋（PVS）、期望窗口、無鎖同形表；能找出連續衝四勝（VCF）的威脅空間靜態搜尋；對活三的
  防守點逐一試下驗證；強制應手延伸；反向無益剪枝、razoring、後期著法剪枝與縮減、fail-high reduction。以 Lazy SMP 使用所有 CPU 核心，瀏覽器
  裡也一樣：額外的 Web Worker 透過共享的 WebAssembly 記憶體一起搜尋。

各部分的細節請看 [wiki](https://github.com/SXKA/tengen/wiki)。

## 建置

需要 Rust 1.97 以上。

```sh
cargo build --release -p tengen-piskvork   # 產生 target/release/pbrain-tengen
cargo test --workspace --release
target/release/pbrain-tengen bench         # 深度 7 的節點數與速度
```

網頁版需要 Node.js 22 與 `wasm32-unknown-unknown` 目標：

```sh
rustup target add wasm32-unknown-unknown
cd web
npm ci
npm run wasm   # 產生 src/engine/tengen.wasm
npm run dev
```

多執行緒版本（`tengen-mt.wasm`）還需要 `rust-src` 元件。部署到 Vercel 時把專案根目錄設成 `web`，建置腳本會安裝 Rust，
`vercel.json` 則送出共享記憶體所需的跨來源隔離標頭。

## 驗證修改

搜尋或估值的修改要能贏棋才會留下。`tengen-arena` 從隨機的均勢開局讓兩個 brain 對戰，每個開局交換先後各下一盤，
並以序貫機率比檢定（SPRT）決定何時停止：

```sh
cargo build --release -p tengen-arena
target/release/tengen-arena --new path/to/new/pbrain-tengen --base path/to/old/pbrain-tengen \
    --games 4000 --turn-ms 100 --sprt 0,10
```

GitHub Actions 的 **SPRT** 工作流程會在分支與 `master` 之間跑同樣的檢定，可選任一規則。**Tune** 工作流程用 `tengen-tune`
在指定規則下產生自我對弈棋譜、擬合新的估值權重，並檢定它是否比現有權重強；每種規則可以有自己的權重
（`crates/engine/src/weights-規則.txt`，沒有就用自由規則的 `weights.txt`）。**Versus Rapfi** 工作流程會編譯
[Rapfi](https://github.com/dhbloo/rapfi)，分別用它的 NNUE 和傳統估值跟 Tengen 對下固定盤數，量出和最強開源引擎的差距。

## 目錄結構

| 路徑 | 內容 |
| --- | --- |
| `crates/engine` | 引擎函式庫：棋盤、棋型、估值、著法生成、搜尋 |
| `crates/piskvork` | `pbrain-tengen`，Gomocup 協定介面 |
| `crates/arena` | `tengen-arena`（對戰與 SPRT）與 `tengen-tune`（自我對弈與調參） |
| `crates/wasm` | WebAssembly 版本（單執行緒與多執行緒），提供純 C ABI |
| `web` | 網頁版：React，引擎跑在 Web Worker |

## 致謝

Tengen 的棋型分類、估值結構與搜尋技巧參考了 dhbloo 等人開發的 [Rapfi](https://github.com/dhbloo/rapfi)，程式碼為獨立實作。

## 授權

GPL-3.0-or-later，見 [LICENSE](LICENSE)。
