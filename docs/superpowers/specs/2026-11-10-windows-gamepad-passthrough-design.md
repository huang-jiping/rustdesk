# Windows 手柄透传设计方案

## 目标与背景

为 RustDesk 增加 Windows 到 Windows 的原生手柄透传：控制端连接的物理 XInput
手柄，在被控端表现为虚拟 Xbox 360 手柄。第一阶段只实现范围明确、可测试的 MVP，
用来验证采集、传输、授权、虚拟设备和清理的完整链路；验证稳定后，再启动自建
Windows 虚拟手柄驱动项目。

开发基线为 RustDesk 1.5.1，提交
`23e5bb471a040d62f0a6ad52591fb30784265b73`，开发分支为
`codex/feature/windows-gamepad-vigem`。

## MVP 目标

- 控制端支持一个物理 XInput 手柄。
- 被控端创建一个虚拟 Xbox 360 手柄。
- 网络协议不暴露 ViGEm 类型，使未来自建驱动只需替换后端。
- 两端均须显式启用：控制端按远端保存开关，被控端使用默认关闭的
  `allow-gamepad` 设置。
- 继续受现有键鼠控制权限约束；权限撤销时立即将虚拟手柄归零并移除。
- 正确处理热插拔、关闭、断网、会话结束、撤权、后端故障和进程退出，不留下持续
  按键或摇杆偏转。
- 功能关闭、不受支持或连接旧版本对端时，现有远控路径保持不变。

## MVP 不包含

- 多于一个物理或虚拟手柄。
- DirectInput-only、PlayStation、Switch、方向盘或通用 HID 设备。
- 震动、LED、耳机音频、电量等反向反馈。
- Windows ARM64、Windows Server、Web、移动端、Linux、macOS 或 Sciter。
- 自动下载、安装、升级或捆绑 ViGEmBus。
- 接受窗口中的独立连接级手柄权限，以及对 `hbb_common` 子模块的修改。
- 保持固定 XInput 玩家编号。
- 在 MVP 分支中开发或签名自建驱动。

## 反目标

- 高频手柄状态不得经过 Flutter、JSON 或 Flutter Rust Bridge。
- 不得把现有键盘、鼠标路径改写为新的公共抽象。
- 不为罕见槽位或重连偏好增加计时器、缓存或跨会话状态。
- ViGEmBus 缺失时不得影响 RustDesk 启动或普通远控。
- 不得静默接受非法槽位、越界轴值、越界扳机值或未定义按钮位。

## 约束

- 以 RustDesk 1.5.1 为稳定基线；protobuf 只能追加字段。
- MVP 目标为 Windows 10/11 x64。其他架构仍须正常构建，但能力为不可用。
- 生产 Rust 代码不使用 `unwrap()` 或 `expect()` 处理正常失败。
- 不跨 `.await` 持锁；ViGEm 阻塞操作放在专用工作线程。
- 对端可控制频率的日志使用 `trace!` 或节流错误日志。
- 新配置键统一从 `base::config::keys` 导入。
- 不运行 `flutter_rust_bridge_codegen`，不提交生成的桥接文件。
- 纯 Rust 后端固定 `vigem-client = "=0.1.4"`，仅在 Windows x64 编译；发布前
  审计源码和 WinAPI 使用并写入 SBOM。
- ViGEmBus 是外部前置条件，RustDesk 只报告缺失，不负责安装。
- 唯一正式支持和测试的驱动是官方最终版 ViGEmBus `v1.22.0` x64。旧版、OEM
  修改版和第三方分支不在支持矩阵内。运行时兼容性以
  `vigem-client::Client::connect()` 和目标就绪结果为准。

## 已考虑的路线

### 1. 纯 Rust ViGEm 客户端——已选定

在 Windows x64 目标依赖中固定 `vigem-client = "=0.1.4"`。该 crate 直接与已安装
的 ViGEmBus 通信，不需要 C/C++ SDK、vcpkg overlay 或额外 DLL，是验证 MVP 最快、
改动最小的路线。代价是它已经停止维护，因此必须锁定、审计并限制在可替换的
`GamepadBackend` 边界内。

### 2. 官方 ViGEmClient 静态库——后备路线

该路线使用官方客户端且没有运行时 DLL，但需要 vcpkg overlay、C/C++ 工具链、
构建脚本和架构分流。只有纯 Rust 后端 spike 失败时才切换。

### 3. 动态加载 ViGEmClient DLL——MVP 不采用

动态加载易于诊断运行时错误，但会增加安全加载、符号校验、架构匹配、签名和打包
检查，首版收益不足以覆盖回归面。

## 用户可见行为

### 被控端

Windows 安全设置页增加“启用手柄透传”，对应 `allow-gamepad`。该键不存在时视为
关闭，而且“完全访问”预设不得自动打开它。驱动缺失或不可用时，普通远控继续工作，
控制端收到结构化状态。

### 控制端

桌面远控工具栏在“键盘”之后增加“手柄”子菜单。仅普通 Windows 到 Windows 远控
会话，且对端声明支持手柄协议时显示。

菜单包含按远端保存、默认关闭的“启用手柄透传”开关，以及本地手柄和远端后端状态。
启用时先请求远端取得所有权和初始化后端；远端返回就绪后才启动 XInput 采集。关闭时
远端归零、移除设备并释放所有权。

Web、移动端、非 Windows、非普通远控和旧端不显示菜单。Web bridge 只增加编译占位，
不调用 Web JS。

## 协议设计

全部改动位于 `libs/base/protos/message.proto`，只使用新字段编号。

- `Features` 追加 `bool gamepad = 4`，表示对端理解协议并包含后端；不代表权限已开或
  外部驱动已安装。
- 顶层 `Message.oneof` 追加 `GamepadMessage gamepad = 35`。
- `GamepadMessage` 包含：
  - `GamepadControl control = 1`：控制端请求启用或禁用。
  - `GamepadInput input = 2`：控制端发送状态。
  - `GamepadStatus status = 3`：被控端发送状态。
- `GamepadInput` 包含：
  - `uint32 slot`：MVP 只接受 `0`，协议保留 `0..3`。
  - `bool connected`。
  - `uint32 buttons`：仅允许 Xbox 360 XInput 按钮掩码。
  - `uint32 left_trigger`、`right_trigger`：`0..255`。
  - 四个 `sint32 thumb_*`：`-32768..32767`。
- 输入包是完整、幂等快照，不传增量、XInput packet number、ViGEm 类型或原始错误码。
- `GamepadStatus` 包含 `slot` 和稳定枚举：`STOPPED`、`READY`、`ACTIVE`、
  `DISABLED_BY_REMOTE`、`DRIVER_MISSING`、`VERSION_MISMATCH`、
  `NO_FREE_SLOT`、`BUSY`、`ACCESS_DENIED`、`INTERNAL_ERROR`。

状态语义：

- `STOPPED`：禁用完成，目标、后端和所有权均已释放。
- `READY`：授权通过、已取得所有权、后端已连接，但当前无虚拟目标。
- `ACTIVE`：虚拟目标已连接并接收状态。
- `DISABLED_BY_REMOTE`：`allow-gamepad` 或键鼠权限关闭。
- 后端错误状态分别表示对应失败；`BUSY` 表示另一连接持有所有权。

每个 `GamepadControl` 都有状态响应。首次附加目标发送 `ACTIVE`；物理手柄拔出并完成
归零/移除后发送 `READY`；撤权主动发送 `DISABLED_BY_REMOTE`；显式禁用成功发送
`STOPPED`。连接已有序，MVP 不增加网络序号。

## 架构

### 控制端采集

在 `src/platform/windows/` 创建 XInput 采集模块。仅在用户开关开启且远端支持时，
专用 OS 线程每 8 ms 轮询 XInput 用户索引 0。

采集线程比较 XInput packet number，只发布变化后的完整快照。首次收到
`ERROR_DEVICE_NOT_CONNECTED` 时发布一次全零断开状态；重连时无条件发布新快照。
线程只写容量受限的 latest-state channel，不直接操作网络。client I/O loop 负责发送，
Flutter FFI 只传低频启停命令。

### 被控端会话与后端

平台无关编排放在 `src/server/gamepad.rs`，ViGEm 实现放在
`src/platform/windows/gamepad.rs` 或清晰的同级模块。

后端边界只暴露平台无关操作：

```rust
trait GamepadBackend: Send {
    fn attach(&mut self, slot: u8) -> ResultType<()>;
    fn update(&mut self, slot: u8, state: &GamepadState) -> ResultType<()>;
    fn detach(&mut self, slot: u8) -> ResultType<()>;
    fn reset_all(&mut self);
}
```

每个获准连接拥有一个 `GamepadSession` 和专用工作线程。网络任务只校验并覆盖单槽
latest-state mailbox；工作线程独占 `vigem-client::Client` 和 `Xbox360Wired`，串行
操作并映射稳定状态码。ViGEm 句柄、报告和数字错误码不得离开 Windows 后端。

进程级原子令牌只允许一个已启用会话。授权通过的 `enabled=true` 请求在物理手柄或
虚拟目标出现前取得令牌；第二个请求返回 `BUSY`。物理手柄拔出时归零并移除目标但
保留令牌。只有显式禁用、撤权、致命错误、连接关闭/Drop 或进程退出才释放令牌，且
由 RAII 在后端清理后释放。

后端运行在活动交互会话的 `--server` 进程，不放入 Session 0 服务；MVP 不增加
portable-service 手柄 IPC。

### 授权条件

输入仅在以下条件全部满足时接受：

- 已授权的普通远控连接。
- Windows x64 后端已编译。
- `allow-gamepad` 明确为 `Y`。
- `peer_keyboard_enabled()` 为 true。
- 当前连接持有手柄所有权。
- `slot == 0` 且所有值合法。

MVP 将手柄视为远程输入并复用键鼠权限。独立连接权限会扩大到共享协议、连接管理
IPC、接受窗口和策略 UI，因此延后。

### 清理规则

显式禁用、撤权、`disable_keyboard`、致命错误、断网、连接关闭/Drop 和进程关闭统一：

1. 停止接受更新。
2. 已有目标时提交全零状态。
3. 移除并释放目标。
4. 释放 ViGEm client。
5. 释放所有权令牌。
6. 连接仍存在时发送最终状态。

物理手柄拔出只归零并移除目标，保留 client 与所有权，并发送 `READY`。

## 配置与 UI

- 在 `libs/base/src/config/keys.rs` 增加 `allow-gamepad` 和按远端保存的
  `gamepad-passthrough`，加入相应同步列表。
- `allow-gamepad` 关闭时必须立即通知活动连接并完整清理，不能等待下一条输入。
- 新增低频 FFI，用于查询和设置会话手柄状态。
- 通过现有 Rust 到 Dart 会话事件流发送 `gamepad_status`。
- 扩展 Dart peer feature 模型，字段缺失默认为 false。
- 只改桌面 `remote_toolbar.dart`，不改共享工具栏。
- 在 `desktop_setting_page.dart` 增加 Windows 被控端开关。
- Web bridge 只加编译占位。
- 新文案追加到 `template.rs` 和全部语言文件；不改既有非空翻译，`it.rs` 新值留空。

## 失败处理

- 驱动缺失、版本不匹配、无空闲槽、访问失败和所有权冲突只禁用手柄。
- 非法输入在进入工作线程前拒绝；重复错误节流。
- attach 失败不得留下半创建目标；update 失败后尝试归零/移除。
- 控制端采集失败时尽可能发送断开状态并停止线程。
- MVP 不自动重试；用户修复问题后手动重新启用。
- 只声明支持 ViGEmBus `v1.22.0`；只有 client API 明确报告时才发送
  `VERSION_MISMATCH`。

## 验证策略

### Rust 测试

- 协议转换：轴、扳机、按钮边界及非法输入。
- 假 XInput provider：首次状态、未变化抑制、断开、packet number 重置后重连、停止。
- latest-state mailbox：突发更新只保留最新快照且不增长。
- mock backend：attach/update、禁用、所有权冲突、撤权和清理顺序。
- capability 缺失时不启动采集、不发送消息。
- 非普通远控和 view-camera 拒绝手柄输入。

Windows 生产路径使用的纯 helper 通过 `#[cfg(any(target_os = "windows", test))]` 在
默认 CI 测试。真实 ViGEm 测试只在 Windows 显式启用并默认忽略。

### Flutter 测试

在 `flutter/test/` 覆盖菜单显示、旧端、远端禁用、后端错误、启停和多会话隔离。

### Windows 验收

- x64 使用固定依赖构建；ARM64 和非 Windows 在禁用功能时仍能构建。
- 未安装 ViGEmBus 时 RustDesk 启动及普通远控正常。
- 两台 Windows 10/11 x64：控制端接物理 XInput 手柄，被控端安装官方
  ViGEmBus `v1.22.0`。
- 在 `joy.cpl` 和实际 XInput 游戏验证按钮、扳机、摇杆与热插拔。
- 验证关闭、撤权、断网、退出、注销、服务重启和驱动故障后虚拟手柄归零并移除。
- 验证新旧版本组合不影响普通远控；第二个会话收到 `BUSY` 且不创建目标。

当前机器在设计调查时没有可用的 `cargo`、`flutter` 或 `dart`，因此这些是实施阶段
门禁，不是已完成证据。

## 交付阶段

1. **后端 spike**：固定 client 连接官方 ViGEmBus `v1.22.0`，创建、更新、移除一个
   target；记录安装包来源、SHA-256、crate 审计和错误映射。失败则评估静态库路线。
2. **协议与纯逻辑**：追加 protobuf，完成校验、状态、所有权和 mock 测试。
3. **被控端**：实现工作线程、ViGEm 后端、授权门禁和清理 hook。
4. **控制端**：实现 XInput 采集、有界传递、能力判断和低频 FFI。
5. **Flutter 与配置**：增加开关、状态 UI、Web stub、本地化和 widget 测试。
6. **集成与打包**：运行 Rust/Flutter 测试、x64/ARM64 构建、兼容性和双机验收。
7. **未来驱动**：在 `GamepadBackend` 后实现自建 VHF/KMDF 或等价后端，完成
   Microsoft 签名、HLK/attestation、Secure Boot/HVCI 测试；协议和 UI 不变。

## 回归面

不可避免改变的既有路径限制为：

- `Cargo.toml`、`Cargo.lock`：Windows x64 后端依赖和 XInput feature。
- `libs/base/protos/message.proto`：追加能力与消息。
- `libs/base/src/config/keys.rs`：两个配置键。
- `src/client/io_loop.rs` 和薄会话/FFI hook：可选采集生命周期。
- `src/server/connection.rs`：薄消息、授权、撤权和关闭 hook。
- `src/flutter.rs`、`src/flutter_ffi.rs`：能力、状态和低频控制。
- 桌面 Flutter 工具栏、设置、Web stub 和本地化表。

现有键鼠注入、portable-service 输入 IPC、移动端、共享工具栏、Web runtime、
Sciter 和非 Windows 路径保持不变。

## 已知风险与限制

- ViGEmBus 和 `vigem-client` 均已停止维护。
- 发布前仍需审查许可证、再分发和商标，即使 MVP 不捆绑驱动。
- Windows ARM64 暂不支持。
- XInput 玩家编号由 Windows 分配，不保证 Player 1。
- 完全发生在两个 8 ms 轮询之间的极短按下和释放可能丢失。
- 强杀进程时依赖 ViGEmBus 清理死进程目标；可控路径仍必须主动归零。
- 未来生产驱动需要 Microsoft Hardware Dev Center 签名、Secure Boot/HVCI 测试及
  持续维护；测试签名不能用于生产。

## 交给实现者决定的细节

- gamepad 模块内部命令和错误类型名称。
- XInput 采集与 ViGEm 后端使用一个 Windows 模块还是两个同级模块。
- Flutter 状态展示复用哪些现有 event/model 类。
- 测试 helper 和 mock 的具体名称，只要保持上述行为和回归约束。
