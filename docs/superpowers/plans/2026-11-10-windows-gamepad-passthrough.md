# Windows 手柄透传实施计划

> **面向执行代理：** 必须使用 `superpowers:subagent-driven-development`（推荐）或
> `superpowers:executing-plans`，严格按任务顺序执行；使用本文的复选框记录进度。

**目标（Goal）：** 在 RustDesk 1.5.1 上实现 Windows x64 单手柄原生透传：控制端
采集一个 XInput 手柄，被控端通过纯 Rust ViGEm 后端创建一个虚拟 Xbox 360 手柄。

**架构（Architecture）：** 高频输入完全留在 Rust：控制端专用线程采集 XInput，
通过有界 latest-state 通道交给现有 client I/O loop；被控端每个启用会话使用专用
worker 串行操作 `vigem-client`。protobuf、Flutter 和配置只承载能力、启停和状态，
`GamepadBackend` 隔离 ViGEm，使未来自建驱动不改变协议和 UI。

**技术栈（Tech Stack）：** Rust、Tokio、protobuf 3、XInput、
`vigem-client = 0.1.4`、Flutter/Dart、Flutter Rust Bridge、Windows 10/11 x64。

**设计文档（Spec）：**
`docs/superpowers/specs/2026-11-10-windows-gamepad-passthrough-design.md`

## 全局约束

- 基线必须保持为 RustDesk `1.5.1` / `23e5bb471a040d62f0a6ad52591fb30784265b73`。
- 只支持 Windows 10/11 x64、XInput 用户索引 0 和协议槽位 0。
- ViGEmBus 只支持官方 `v1.22.0` x64，且不由 RustDesk 下载、安装或捆绑。
- `vigem-client` 必须固定为精确版本 `=0.1.4`，只在 Windows x64 目标依赖中出现。
- protobuf 只追加字段：`Features.gamepad = 4`、`Message.gamepad = 35`。
- `allow-gamepad` 和控制端 peer 选项默认关闭。
- 手柄输入继续受 `peer_keyboard_enabled()` 约束。
- 高频状态不得经过 Flutter、JSON 或 FRB；不得使用无界队列。
- 不运行 `flutter_rust_bridge_codegen`，不提交生成桥接文件。
- 不修改 `hbb_common` 子模块、移动端、Sciter、共享 Flutter 工具栏或 Web JS runtime。
- 生产 Rust 代码遵守仓库的 `unwrap`、日志、Tokio、import 和最小改动规则。
- 每个任务提交前运行其局部测试；最终任务运行全量可用验证和回归面检查。

## 文件结构

### 新建文件

- `src/server/gamepad.rs`：平台无关状态校验、后端 trait、所有权、mailbox、worker、
  会话生命周期和 mock 测试。
- `src/platform/windows/gamepad_capture.rs`：控制端 XInput provider、8 ms 采集状态机和
  bounded channel。
- `src/platform/windows/gamepad_backend.rs`：`vigem-client` 适配、错误映射和 ignored
  实机测试。
- `flutter/test/gamepad_toolbar_test.dart`：桌面菜单、能力、状态和会话隔离测试。

### 修改文件

- `Cargo.toml`、`Cargo.lock`：Windows x64 精确依赖及 XInput feature。
- `libs/base/protos/message.proto`：能力、控制、输入和状态消息。
- `libs/base/src/config/keys.rs`：`allow-gamepad` 与 `gamepad-passthrough`。
- `src/server.rs`、`src/platform/windows.rs`：只增加模块声明。
- `src/server/connection.rs`：薄协议分派、授权、状态发送、撤权和关闭 hook。
- `src/ipc.rs`：设置关闭后立即撤销活动手柄会话的薄 hook。
- `src/client.rs`：新增低频 `Data::GamepadEnabled(bool)`。
- `src/client/io_loop.rs`：能力解析、启停控制、capture channel 和网络发送。
- `src/ui_session_interface.rs`：向 client I/O loop 发送低频启停命令。
- `src/flutter.rs`、`src/flutter_ffi.rs`：能力/状态事件和低频 FFI。
- `flutter/lib/models/model.dart`：peer capability 与会话状态。
- `flutter/lib/desktop/widgets/remote_toolbar.dart`：桌面 Gamepad 子菜单。
- `flutter/lib/desktop/pages/desktop_setting_page.dart`：Windows 被控端全局开关。
- `flutter/lib/web/bridge.dart`：unsupported/no-op 编译占位。
- `src/lang/*.rs`：只追加新 key；`it.rs` 值为空。

## 重点复核项

1. `allow-gamepad` 或键鼠权限关闭时，活动目标必须立即归零、移除并释放所有权；
   Task 3 和 Task 4 的撤权测试覆盖。
2. 网络关闭、Drop 或 worker 错误时不得留下卡键或摇杆偏转；Task 3 的清理顺序测试
   和 Task 8 双机断网验收覆盖。
3. 旧端或 capability 缺失时不得显示 UI、启动采集或影响普通远控；Task 5、Task 7
   覆盖。
4. 两个会话并发启用时只能第一个取得所有权，第二个稳定返回 `BUSY`；Task 3 覆盖。
5. Windows ARM64、非 Windows 及未安装驱动场景必须继续构建/启动；Task 2 和 Task 8
   覆盖。

---

### Task 1：追加协议并建立平台无关状态模型

**文件：**

- 修改：`libs/base/protos/message.proto:103-107,1005-1040`
- 新建：`src/server/gamepad.rs`
- 修改：`src/server.rs:73`

**接口：**

- 产出：protobuf `GamepadControl`、`GamepadInput`、`GamepadStatus`、
  `GamepadMessage` 和 `GamepadStatusKind`。
- 产出：`GamepadState::try_from_input(input: &GamepadInput) -> Result<GamepadState>`。
- 产出：`trait GamepadBackend: Send`，提供 `attach`、`update`、`detach`、
  `reset_all`。
- 后续任务只使用 `GamepadState` 和稳定状态枚举，不直接使用 ViGEm 类型。

- [ ] **Step 1：在 `src/server/gamepad.rs` 写协议校验失败测试**

  增加测试：`accepts_xinput_boundary_values`、`rejects_nonzero_slot`、
  `rejects_trigger_above_255`、`rejects_axis_out_of_range`、
  `rejects_undefined_button_bits`。断言合法边界精确保留，非法值返回错误且不 panic。

- [ ] **Step 2：运行测试并确认先失败**

  运行：`cargo test --locked --lib server::gamepad::tests -- --nocapture`

  预期：因模块、protobuf 类型或转换函数不存在而编译失败。

- [ ] **Step 3：向 `message.proto` 追加精确协议**

  添加设计文档规定的字段编号和枚举；不得改动或复用既有编号。`GamepadInput` 的轴
  使用 `sint32`，扳机和按钮使用 `uint32`。

- [ ] **Step 4：实现最小 `GamepadState`、校验和后端 trait**

  `GamepadState` 使用 `u16 buttons`、`u8` 扳机和 `i16` 轴；转换时显式检查 slot、
  范围；包含未定义按钮位时返回错误。此任务不创建线程、所有权或 ViGEm 后端。

- [ ] **Step 5：运行协议与单元测试**

  运行：

  ```powershell
  cargo check --locked -p base
  cargo test --locked --lib server::gamepad::tests -- --nocapture
  ```

  预期：全部通过，protobuf 生成代码只出现在 `target/`。

- [ ] **Step 6：提交**

  ```powershell
  git add 'libs/base/protos/message.proto' 'src/server.rs' 'src/server/gamepad.rs'
  git commit -m 'feat(gamepad): 添加手柄协议与状态校验'
  ```

### Task 2：验证纯 Rust ViGEm 后端

**文件：**

- 修改：`Cargo.toml:113-155`
- 修改：`Cargo.lock`
- 新建：`src/platform/windows/gamepad_backend.rs`
- 修改：`src/platform/windows.rs:99-104`

**接口：**

- 消费：Task 1 的 `GamepadBackend`、`GamepadState` 和状态枚举。
- 产出：`VigemBackend::connect() -> Result<VigemBackend, GamepadBackendError>`。
- 产出：`map_vigem_error(error: vigem_client::Error) -> GamepadStatusKind`。
- 产出：`create_gamepad_backend() -> Result<Box<dyn GamepadBackend>, ...>`；仅
  Windows x64 返回 ViGEm 实现，其他目标不编译该实现。

- [ ] **Step 1：写后端映射与架构门控测试**

  测试 `XGamepad` 字段映射、`BusNotFound -> DRIVER_MISSING`、
  `BusVersionMismatch -> VERSION_MISMATCH`、`NoFreeSlot -> NO_FREE_SLOT`。
  添加默认忽略的 `connects_to_vigem_1_22_0` 实机测试，要求环境变量
  `RUSTDESK_TEST_VIGEM=1`。

- [ ] **Step 2：运行测试并确认先失败**

  运行：`cargo test --locked --lib gamepad_backend -- --nocapture`

  预期：模块和精确依赖尚不存在。

- [ ] **Step 3：添加精确 Windows x64 依赖**

  在 `cfg(all(target_os = "windows", target_arch = "x86_64"))` 依赖段添加
  `vigem-client = "=0.1.4"`。此任务不启用 XInput feature，避免后端任务提前改变控制端
  采集依赖。

- [ ] **Step 4：实现最小 ViGEm 适配器**

  `VigemBackend` 独占 client 和最多一个 `Xbox360Wired`。attach 执行 plugin 和
  wait-ready；update 只提交完整报告；detach 先提交零报告再移除；Drop 复用同一清理。
  不实现震动回调。

- [ ] **Step 5：运行无驱动测试与可选实机 spike**

  运行：

  ```powershell
  cargo test --locked --lib gamepad_backend -- --nocapture
  cargo check --locked --target aarch64-pc-windows-msvc
  $env:RUSTDESK_TEST_VIGEM = '1'
  cargo test --locked --lib connects_to_vigem_1_22_0 -- --ignored --nocapture
  Remove-Item Env:RUSTDESK_TEST_VIGEM
  ```

  预期：普通测试不要求驱动；ARM64 不引入 `vigem-client`；实机测试能创建、更新、
  归零和移除目标。若 spike 失败，停止后续任务并报告，不得绕过到产品代码。

- [ ] **Step 6：记录依赖证据并提交**

  在设计文档或后续发布记录中记录官方 `v1.22.0` 来源、SHA-256 和 crate 审计结论。

  ```powershell
  git add 'Cargo.toml' 'Cargo.lock' 'src/platform/windows.rs' 'src/platform/windows/gamepad_backend.rs'
  git commit -m 'feat(gamepad): 接入纯 Rust ViGEm 后端'
  ```

### Task 3：实现被控端会话、所有权和 worker 生命周期

**文件：**

- 修改：`src/server/gamepad.rs`

**接口：**

- 产出：`GamepadSession::enable(status_sender, backend_factory) -> GamepadStatusKind`。
- 产出：`GamepadSession::handle_input(&self, input: GamepadInput) -> Result<()>`。
- 产出：`GamepadSession::disable(&self, final_status: GamepadStatusKind)`。
- 产出：`revoke_active_gamepad()`，供设置与权限 hook 立即清理唯一 owner。
- 产出：进程级 RAII `GamepadOwnership`，同一时刻最多一个 owner。

- [ ] **Step 1：用 mock backend 写生命周期失败测试**

  覆盖：首次 enable 返回 READY；第二会话返回 BUSY；connected 输入执行
  attach→update→ACTIVE；物理断开执行 zero→detach→READY 但保留 owner；显式禁用执行
  zero→detach→client drop→owner release→STOPPED；worker update 错误完整清理。

- [ ] **Step 2：写 latest-state mailbox 测试**

  连续写入 1000 个状态，断言内存结构容量恒定且 worker 最终观察到最后一个完整状态；
  禁止使用 `mpsc::unbounded_channel`。

- [ ] **Step 3：运行测试并确认失败**

  运行：`cargo test --locked --lib server::gamepad::tests -- --nocapture`

  预期：会话、worker、owner 和 mailbox 尚不存在。

- [ ] **Step 4：实现最小状态机**

  使用 `Mutex<Option<WorkerCommand>> + Condvar` 或等价容量为一的覆盖式 mailbox。
  worker 独占 backend；Tokio/网络侧只校验和覆盖状态，不调用阻塞后端。所有权只在完整
  清理之后释放。Drop 仅发出停止并唤醒，不在 async 线程无限 join。

- [ ] **Step 5：运行测试并检查日志策略**

  运行：`cargo test --locked --lib server::gamepad::tests -- --nocapture`

  预期：全部通过；高频成功路径无 debug 日志，重复故障使用节流日志。

- [ ] **Step 6：提交**

  ```powershell
  git add 'src/server/gamepad.rs'
  git commit -m 'feat(gamepad): 实现手柄会话生命周期'
  ```

### Task 4：接入被控端连接、权限和设置撤销

**文件：**

- 修改：`libs/base/src/config/keys.rs:37-55,273-300`
- 修改：`src/server/connection.rs:258-267,803-821,3199-3395,5308,6766-6783`
- 修改：`src/ipc.rs:1074-1094`
- 测试：`src/server/connection.rs` 邻近 `#[cfg(test)]` 模块

**接口：**

- 消费：Task 3 的 `GamepadSession` 和 `revoke_active_gamepad()`。
- 产出：`OPTION_ALLOW_GAMEPAD = "allow-gamepad"` 和
  `OPTION_GAMEPAD_PASSTHROUGH = "gamepad-passthrough"`。
- 产出：`Connection` 对 `GamepadMessage` 的控制/输入分派与状态回复。

- [ ] **Step 1：写授权与连接类型失败测试**

  覆盖：`allow-gamepad` 空值/非 Y 拒绝；键鼠权限关闭拒绝；view-camera、文件传输和
  未授权连接拒绝；普通远控且权限齐全才允许 enable。

- [ ] **Step 2：写立即撤权与关闭清理测试**

  覆盖 `SwitchPermission("keyboard", false)`、`disable_keyboard`、
  `allow-gamepad` 从 Y 改为空、`on_close()` 和 Drop；断言不等待下一条输入即调用完整
  清理并发送 `DISABLED_BY_REMOTE`（连接仍存在时）。

- [ ] **Step 3：运行测试并确认失败**

  运行：`cargo test --locked --lib server::connection::tests -- --nocapture`

- [ ] **Step 4：添加配置键和薄连接 hook**

  `Connection` 只保存 `Option<GamepadSession>`，协议分支先检查会话类型、授权、
  `allow-gamepad` 和 `peer_keyboard_enabled()`，再调用 session。不得把 gamepad 加入
  view-camera 兼容白名单。

- [ ] **Step 5：接入设置撤销**

  `ipc::Data::Options(Some(value))` 是整表替换，普通 UI 用删除键表达关闭。成功更新后
  读取有效 `Config::get_option(allow-gamepad)`（包含默认/强制覆盖），值不是 `Y` 时
  调用 `revoke_active_gamepad()`；不得把缺键当作局部更新的“未修改”。不要为此修改
  通用 permission enum 或 `hbb_common`。

- [ ] **Step 6：运行测试**

  ```powershell
  cargo test --locked --lib server::connection::tests -- --nocapture
  cargo test --locked --lib server::gamepad::tests -- --nocapture
  ```

- [ ] **Step 7：提交**

  ```powershell
  git add 'libs/base/src/config/keys.rs' 'src/server/connection.rs' 'src/ipc.rs'
  git commit -m 'feat(gamepad): 接入远端授权与连接清理'
  ```

### Task 5：实现控制端 XInput 采集和网络发送

**文件：**

- 新建：`src/platform/windows/gamepad_capture.rs`
- 修改：`src/platform/windows.rs:99-104`
- 修改：`src/client.rs:5006-5040`
- 修改：`src/client/io_loop.rs:95-135,293-331,800-850,1745-1824,2594-2601`
- 修改：`src/ui_session_interface.rs:62-84,1789-1798`

**接口：**

- 产出：`trait XInputProvider { fn get_state(&self, user_index: u32) -> ... }`。
- 产出：`GamepadCapture::start(sender) -> Result<GamepadCapture>` 与 `stop()`。
- 产出：`Data::GamepadEnabled(bool)`，只传低频启停意图。
- 产出：`Session::set_gamepad_enabled(enabled: bool)`。
- 消费：Task 1 的协议类型与远端 `Features.gamepad`。

- [ ] **Step 1：用 fake provider 写采集状态机失败测试**

  覆盖首次连接、packet number 不变不发送、状态变化发送、首次断开只发一次全零状态、
  重插 packet number 回到 0 仍发送、stop 发送最终断开并退出。

- [ ] **Step 2：写 client I/O 能力与启停失败测试**

  覆盖 capability 缺失/false 不发送 control、不启动线程；true 时先发送 enable control，
  仅收到 READY 后启动采集；STOPPED 或错误状态停止采集。

- [ ] **Step 3：运行测试并确认失败**

  ```powershell
  cargo test --locked --lib gamepad_capture -- --nocapture
  cargo test --locked --lib client::io_loop::tests -- --nocapture
  ```

- [ ] **Step 4：实现 8 ms 采集和容量受限传递**

  为根 crate 现有 `winapi` 增加直接调用 XInput 所需的 `xinput` feature，不依赖
  `vigem-client` 的传递 feature。

  仅轮询 XInput 用户索引 0。采集线程使用容量为一或覆盖式 latest-state 传递；不在
  capture 线程直接访问 peer。Drop/stop 必须可重复且不 panic。

- [ ] **Step 5：接入 client I/O loop**

  `ParsedPeerInfo` 增加 `support_gamepad: bool`。`Data::GamepadEnabled(true)` 先发送
  control；收到 READY 再启动 capture；capture channel 分支把快照封装为
  `Message.gamepad.input` 并 `peer.send()`。连接结束统一 stop。

- [ ] **Step 6：运行测试**

  ```powershell
  cargo test --locked --lib gamepad_capture -- --nocapture
  cargo test --locked --lib client::io_loop::tests -- --nocapture
  cargo test --locked --release --lib --features flutter
  ```

- [ ] **Step 7：提交**

  ```powershell
  git add 'src/platform/windows.rs' 'src/platform/windows/gamepad_capture.rs' 'src/client.rs' 'src/client/io_loop.rs' 'src/ui_session_interface.rs'
  git commit -m 'feat(gamepad): 添加 XInput 采集与发送链路'
  ```

### Task 6：增加低频 FFI、能力和状态模型

**文件：**

- 修改：`src/flutter_ffi.rs:252-270,364-384`
- 修改：`src/flutter.rs:900-939`
- 修改：`flutter/lib/models/model.dart:220-245,330-362,1460-1465,4506-4524`
- 修改：`flutter/lib/web/bridge.dart`

**接口：**

- 产出：`session_get_gamepad_enabled(session_id) -> SyncReturn<bool>`。
- 产出：`session_set_gamepad_enabled(session_id, enabled)`。
- 产出：Rust→Dart `gamepad_status` 事件，字段为 `status`、`slot`、
  `local_connected`。
- 产出：Dart peer feature `gamepad`，缺失时 false。

- [ ] **Step 1：写 Rust FFI 会话路由测试或最小邻近测试**

  断言无效 SessionID 安全返回 false/不操作；有效 session 只发送
  `Data::GamepadEnabled`，不传高频状态。

- [ ] **Step 2：写 Dart model 失败测试**

  在 `flutter/test/gamepad_toolbar_test.dart` 先加入 model 层测试：缺失 capability 为
  false；`gamepad_status` 只更新对应 SessionID；未知状态安全降级为 INTERNAL_ERROR。

- [ ] **Step 3：运行并确认失败**

  ```powershell
  cargo test --locked --lib --features flutter gamepad
  Set-Location -LiteralPath 'flutter'
  flutter test 'test/gamepad_toolbar_test.dart'
  Set-Location -LiteralPath '..'
  ```

- [ ] **Step 4：实现低频 FFI 与状态事件**

  复用现有 session sender 和事件流。`src/flutter.rs` 的 peer features JSON 追加
  `gamepad`；不得改变既有 feature 的默认值。

- [ ] **Step 5：添加 Web 编译 stub**

  `flutter/lib/web/bridge.dart` 提供相同方法签名，查询恒为 false、设置为 no-op；不得
  调用 JS 或暴露 Web UI。

- [ ] **Step 6：运行测试**

  ```powershell
  cargo test --locked --release --lib --features flutter
  Set-Location -LiteralPath 'flutter'
  flutter test 'test/gamepad_toolbar_test.dart'
  Set-Location -LiteralPath '..'
  ```

- [ ] **Step 7：提交**

  ```powershell
  git add 'src/flutter_ffi.rs' 'src/flutter.rs' 'flutter/lib/models/model.dart' 'flutter/lib/web/bridge.dart' 'flutter/test/gamepad_toolbar_test.dart'
  git commit -m 'feat(gamepad): 暴露会话状态与控制接口'
  ```

### Task 7：实现桌面 UI、被控端设置和本地化

**文件：**

- 修改：`flutter/lib/desktop/widgets/remote_toolbar.dart:803-854,2547-2595`
- 修改：`flutter/lib/desktop/pages/desktop_setting_page.dart:1201-1277`
- 修改：`flutter/test/gamepad_toolbar_test.dart`
- 修改：`src/lang/template.rs`、`src/lang/en.rs`、其余 `src/lang/*.rs`

**接口：**

- 消费：Task 6 的 capability、状态 model 和 FFI。
- 产出：桌面 `_GamepadMenu`，仅 Windows→Windows 普通远控且 capability=true 显示。
- 产出：Windows 安全设置 `allow-gamepad` 复选框，缺失时关闭。

- [ ] **Step 1：扩展 widget 失败测试**

  覆盖：非 Windows、非 Windows peer、旧端、非普通连接均隐藏；远端禁用时显示但不可
  开启；READY 可开启；ACTIVE 显示活动；DRIVER_MISSING 显示错误并把开关恢复为关；
  两个 SessionID 状态互不污染。

- [ ] **Step 2：运行测试并确认失败**

  ```powershell
  Set-Location -LiteralPath 'flutter'
  flutter test 'test/gamepad_toolbar_test.dart'
  Set-Location -LiteralPath '..'
  ```

- [ ] **Step 3：实现桌面 Gamepad 子菜单**

  放在 Keyboard 后、Chat 前，复用 `_IconSubmenuButton` 和 `CkbMenuButton`。不得修改
  `flutter/lib/common/widgets/toolbar.dart`。菜单只显示本地连接和远端后端状态，不在
  Dart 中轮询或传输手柄数据。

- [ ] **Step 4：实现被控端默认关闭设置**

  在 Windows Permissions 卡片中加入 `allow-gamepad`。不要复用使“完全访问”自动
  开启的 `fakeValue=true` 路径。

- [ ] **Step 5：追加最少本地化 key**

  使用句式大小写 key，例如 `Gamepad`、`Enable gamepad passthrough`、
  `Remote gamepad driver is unavailable`、`Gamepad is disabled on the remote device`。
  只追加空缺条目；`it.rs` 必须为空，品牌和技术词不翻译。

- [ ] **Step 6：运行 Flutter 测试和 analyze**

  ```powershell
  Set-Location -LiteralPath 'flutter'
  flutter test 'test/gamepad_toolbar_test.dart'
  flutter test --timeout 60s
  flutter analyze
  Set-Location -LiteralPath '..'
  ```

- [ ] **Step 7：检查中文和编码后提交**

  ```powershell
  git diff --check
  git add 'flutter/lib/desktop/widgets/remote_toolbar.dart' 'flutter/lib/desktop/pages/desktop_setting_page.dart' 'flutter/test/gamepad_toolbar_test.dart' 'src/lang'
  git commit -m 'feat(gamepad): 添加桌面手柄开关与状态界面'
  ```

### Task 8：全量验证、兼容性和回归面最小化

**文件：**

- 检查：本计划涉及的全部修改文件
- 可选修改：`.github/workflows/flutter-build.yml`，仅当新增 feature 没有现有 CI
  覆盖时添加精确测试命令
- 更新：`docs/superpowers/specs/2026-11-10-windows-gamepad-passthrough-design.md`
  中的实际验证结果或已知限制（不得把计划结果伪装成已验证事实）

**接口：**

- 消费：Task 1–7 的完整实现。
- 产出：可审计的自动测试、构建、双机验收结果和最终回归面说明。

- [ ] **Step 1：运行格式、Rust 和 Flutter 验证**

  ```powershell
  cargo fmt --all -- --check
  cargo test --locked --workspace --no-fail-fast
  cargo test --locked --release --lib --features flutter
  Set-Location -LiteralPath 'flutter'
  flutter test --timeout 60s
  flutter analyze
  Set-Location -LiteralPath '..'
  git diff --check
  ```

  每个外部命令后立即检查 `$LASTEXITCODE`；不得因后续命令成功而掩盖前一失败。

- [ ] **Step 2：验证目标矩阵**

  - Windows x64：feature 开启并能连接 ViGEmBus `v1.22.0`。
  - Windows ARM64：正常构建，capability=false，不包含 `vigem-client`。
  - 非 Windows：默认 workspace 测试通过。
  - 未安装驱动：RustDesk 启动和普通远控正常，状态为 DRIVER_MISSING。
  - 新旧端任意组合：普通远控正常，旧端不显示/不发送手柄功能。

- [ ] **Step 3：执行两台 Windows x64 手工验收**

  使用 `joy.cpl` 和实际 XInput 游戏验证按键、扳机、轴、热拔插、关闭、撤权、断网、
  退出、注销、服务重启、驱动故障、第二会话 BUSY。逐项记录实际结果和日志。

- [ ] **Step 4：执行强制回归面检查**

  逐个检查现有修改文件：删除无关重构、格式化和顺手修复；确认 feature off 仍走旧路径；
  确认没有修改 `hbb_common`、移动端、Sciter、共享 toolbar、Web JS 或 portable-service
  输入 IPC；确认测试只覆盖报告行为。

- [ ] **Step 5：检查依赖与许可证**

  记录 `vigem-client 0.1.4`、ViGEmBus `v1.22.0` 的来源、许可证、SHA-256、SBOM 和
  EOL 风险；确认安装包未进入 RustDesk 产物。

- [ ] **Step 6：按实际结果提交最终验证调整**

  若没有代码调整则不创建空提交；若只更新验证文档：

  ```powershell
  git add 'docs/superpowers/specs/2026-11-10-windows-gamepad-passthrough-design.md'
  git commit -m 'docs(gamepad): 记录手柄透传验证结果'
  ```

## 最终验收标准

- 一个物理 XInput 手柄在被控端表现为一个 Xbox 360 Controller。
- 功能默认关闭，且受 `allow-gamepad` 和键鼠权限双重约束。
- 禁用、撤权、拔出、断网和退出后无卡键、无轴漂移、无残留 target。
- 驱动缺失、版本错误、BUSY 和后端错误不影响普通远控。
- 旧端、ARM64、非 Windows、Web 和移动端保持既有行为。
- 自动测试、Windows 构建和双机验收均有可复查证据；未执行项明确标注为未验证。
