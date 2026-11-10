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
改动最小的路线。其依赖的 ViGEm 项目已 EOL，第三方 Rust crate 的后续维护也无
保证，因此必须锁定、审计并限制在可替换的 `GamepadBackend` 边界内。

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
  `gamepad-passthrough`。前者加入 `KEYS_SETTINGS`；后者沿用直接写入 peer options
  的保存路径，不扩展自定义客户端 default/override 分类列表。
- `allow-gamepad` 关闭时必须立即通知活动连接并完整清理，不能等待下一条输入。
- 设置 IPC 的 Options 是整表替换，普通 UI 用删除键关闭。成功更新后按有效
  `Config::get_option(allow-gamepad)` 判断撤销，保留默认/强制覆盖语义。
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
- MVP 不自动重连或重新启用；用户修复问题后手动启用。底层完整报告仅在首次返回
  `WinError(259)` 时等待 1 ms 并重提一次，第二次失败仍按后端故障清理。
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

- ViGEmBus 已 EOL；第三方 `vigem-client` 的维护状态不能由 ViGEm 官方 EOL 推定，
  本次查询未见该仓库归档或其作者的 EOL 声明，后续维护仍无保证。
- 发布前仍需审查许可证、再分发和商标，即使 MVP 不捆绑驱动。
- Windows ARM64 暂不支持。
- XInput 玩家编号由 Windows 分配，不保证 Player 1。
- 完全发生在两个 8 ms 轮询之间的极短按下和释放可能丢失。
- 强杀进程后的目标清理没有保证；crate 文档警告析构未执行可能遗留设备。可控路径
  仍必须主动归零，强杀行为须另外验收。
- 未来生产驱动需要 Microsoft Hardware Dev Center 签名、Secure Boot/HVCI 测试及
  持续维护；测试签名不能用于生产。

## 交给实现者决定的细节

- gamepad 模块内部命令和错误类型名称。
- XInput 采集与 ViGEm 后端使用一个 Windows 模块还是两个同级模块。
- Flutter 状态展示复用哪些现有 event/model 类。
- 测试 helper 和 mock 的具体名称，只要保持上述行为和回归约束。

## Task 2 依赖审计证据（2026-11-10）

- 官方驱动来源：[ViGEmBus v1.22.0 release](https://github.com/nefarius/ViGEmBus/releases/tag/v1.22.0)，
  资产为 [ViGEmBus_1.22.0_x64_x86_arm64.exe](https://github.com/nefarius/ViGEmBus/releases/download/v1.22.0/ViGEmBus_1.22.0_x64_x86_arm64.exe)。
  从官方 GitHub release API 确认资产名及大小 `6278576` 字节，下载到临时目录后用
  `Get-FileHash -Algorithm SHA256` 实测为
  `89220a7865076b342892f98865f3499fb7c4cfd673159e89d352c360fd014c6a`。
  `Get-AuthenticodeSignature` 返回 `Valid`，签名主体为
  `Nefarius Software Solutions e.U.`。只下载校验，没有执行安装包或安装驱动。
- [vigem-client 0.1.4](https://docs.rs/vigem-client/0.1.4/vigem_client/) 精确固定于
  Windows x64 target 依赖段；crate 压缩包实测 SHA-256 与 Cargo.lock checksum 均为
  `b857e6f99efe1e1eb1e4dfb035de8ae7ec8ec56bd1928edcbd7c6e4427634d52`。
  发布包 `.cargo_vcs_info.json` 记录源码提交
  `acbc38bbc159e315622b5dbd9b41048fc9f3164c`，对应
  [上游源码](https://github.com/CasualX/vigem-client/tree/acbc38bbc159e315622b5dbd9b41048fc9f3164c)。
- 审阅发布包 `Cargo.toml`、`license.txt`、`client.rs`、`event.rs`、`bus.rs` 和 `x360.rs`
  中使用路径。普通依赖仅 `winapi 0.3`；`rusty-xinput`、`urandom` 仅为其开发依赖，
  不引入 RustDesk。crate 无 build script、C/C++ SDK 或额外 DLL；未启用两个 unstable
  feature。其自身 `winapi/xinput` 仅提供报告结构，Task 2 未修改 RustDesk 的采集
  feature。MIT 许可要求保留版权和许可声明，发布 SBOM/通知应列出该 crate 与 winapi；
  此处是源码与许可清单核查，不代替发布法律审查。
- 连接使用 SetupAPI 枚举接口及 `CreateFileW`，通过 `DeviceIoControl` 和
  `GetOverlappedResult` 同步等待；目标及 client 使用 Drop 关闭事件、移除目标和关闭
  句柄。因此后续调用必须放在专用 OS 工作线程，不能在 async 网络任务中直接调用。
  后端只公开 `GamepadBackend` 与稳定状态错误，不泄漏 ViGEm 句柄、报告或数字错误码。
- 已知上游限制：`plugin` 遍历序列号并将全部失败折叠成 `NoFreeSlot`；
  `Event::new` 只用 debug assertion 检查事件创建；`wait_ready` 接受旧驱动的特定错误，
  `connect` 仅检查协议版本，不能证明安装的是正式 v1.22.0。实机环境须另外核对驱动。
  `Xbox360Wired` 的 Drop 忽略 unplug 错误，后端主动归零和移除并报告失败，Drop 仅作
  最后兜底。上游文档警告强杀进程可能遗留设备，因此仍需后续实机验收，不能宣称已
  验证驱动会清理所有强杀场景。
- 已验证初始失败根因：首次完整报告返回 `WinError(259)`。官方 v1.22.0 的
  [SubmitReportImpl](https://github.com/nefarius/ViGEmBus/blob/v1.22.0/sys/XusbPdo.cpp#L907-L956)
  在没有待处理 USB 输入请求时于复制/缓存报告之前返回；
  [官方客户端](https://github.com/nefarius/ViGEmClient/blob/master/src/ViGEmClient.cpp#L921-L967)
  对该类错误返回成功，crate 0.1.4 则返回 `WinError`。临时实机诊断显示第一次 259
  后立即及 20 ms 后均未收到目标状态，重提后才读回精确报告；另一组无打印/读取干扰
  的采样在第 1 次 259 后，于第 2 次（1 ms 间隔）成功。诊断代码已删除。
- 经主会话批准，提交函数仅在首个 259 后等待 1 ms，原样重提一次；不吞掉失败，第二次
  或其他错误原样传播。非零更新与 detach/Drop 的零报告使用同一函数，不增加持久状态。
  先取得 helper 缺失的 RED（退出 101），再实现并取得普通测试 `5 passed; 1 ignored`
  （退出 0）。最终 ignored `connects_to_vigem_1_22_0` 退出 0，验证首次完整更新与
  XInput 精确读回、归零读回、移除、重新附加和 Drop 清理后的设备不可见。
  测试读取使用 crate 已带入的 winapi feature union，未增加根采集 feature。
- 此次实机上 ViGEmBus 正在运行，PnP 与 `ViGEmBus.sys` 版本均为 `1.21.442.0`。
  官方 v1.22.0 release 明示驱动版本未变，文件版本与之相容，但不能仅据文件版本
  证明安装包来源；此次未运行安装器。ARM64 依赖图确认无 vigem-client，但完整
  `cargo check --target aarch64-pc-windows-msvc` 因缺少 ARM64 C 编译器阻塞；根库
  测试唯一失败为已有 ACL 目录清理拒绝访问。这些不属于后端普通测试或 spike 通过。

## Task 8 实际验证结果（2026-11-10）

结论：实现和可运行的专项检查已验证，但完整 native Flutter 构建、全量回归和双机
验收尚未通过。以下是本次重新执行的实际结果，不替代上文的目标与验收门禁。

环境：Windows x64、PowerShell 7.6.9、Cargo/Rust 1.99.1、Flutter 3.47.9、
Dart 3.13.7。Cargo 设置 `LIBCLANG_PATH=C:\Program Files\LLVM\bin` 和
`VCPKG_ROOT=C:\Users\huang\.codex\toolchains\vcpkg`。命令使用已确认的绝对工具路径。
原始命令、时间、退出码与日志保存在本地忽略目录
`.superpowers/sdd/2026-11-10-windows-gamepad-passthrough/task-8-*`；未运行 FRB codegen，
未生成桥接替身，未变更或重装驱动。

| 命令/检查 | 实际结果 | 退出码 |
| --- | --- | --- |
| `cargo fmt --all -- --check` | 既有格式差异，且缺少 `src/ui/inline.rs`，未完成全树格式检查 | 1 |
| 四个新 Rust 模块的 `rustfmt --edition 2021 --check` | 通过 | 0 |
| `cargo check --locked -p base` | 通过，包含追加 protobuf 生成/编译 | 0 |
| `cargo test --locked --workspace --no-fail-fast` | 编译受阻：portable packer 缺少 `data.bin`、`app_metadata.toml`，未进入完整测试 | 101 |
| `cargo test --locked --lib` | 237 passed / 1 failed / 1 ignored | 101 |
| `cargo test --locked --release --lib --features flutter` | 缺 `src/bridge_generated.rs`（E0583）及 5 个连带 E0277，未执行测试 | 101 |
| `cargo test --locked --lib gamepad -- --nocapture` | 29 passed / 0 failed / 1 ignored | 0 |
| `RUSTDESK_TEST_VIGEM=1` 下的 ignored `connects_to_vigem_1_22_0` | 1 passed：完整报告与归零读回、detach、再次 attach、Drop 后设备不可见 | 0 |
| `cargo check --locked --target aarch64-pc-windows-msvc` | 原生依赖缺少 ARM64 `cl.exe`，未完成项目检查 | 101 |
| ARM64 `cargo tree --locked --target aarch64-pc-windows-msvc -e features -i winapi` | 不含 `vigem-client v` 或 `winapi feature "xinput"` | 0 |
| `flutter test --no-pub --no-test-assets test/gamepad_toolbar_test.dart` | 6 passed；是产品模型/决策测试，未渲染真实菜单或调用 native FFI | 0 |
| `flutter test --no-pub --no-test-assets --timeout 60s` | 58 passed / 1 skipped / 17 个测试文件加载失败 | 1 |
| `flutter analyze --no-pub` | 443 issues | 1 |
| `dart analyze`（gamepad model/test） | No issues found | 0 |
| 本地化与严格 UTF-8 检查 | 54 张语言表有效；en 无冗余项，其他 53 表各追加 10 个唯一 key，旧条目未变，it/template 留空 | 0 |
| `git diff --check 52eef50c8` | 通过 | 0 |

根库唯一失败仍为未改动的
`platform::windows::installer_handoff::directory::tests::protected_directory_enforces_creator_permissions`：
`directory.rs:115:36` 清理目录时报 `Os { code: 5, kind: PermissionDenied, message: "Access is denied." }`。
全量 Flutter 的加载错误仍包括缺失 `generated_bridge.dart`、`DialogTheme`/`TabBarTheme`
与 SDK 类型不匹配，以及 `google_fonts 6.2.1` 的 FontWeight 常量求值错误。没有把这些
环境/既有问题并入本功能修复，也没有将专项通过表述为全量通过。

### 目标矩阵与未执行验收

| 项目 | 状态与证据边界 |
| --- | --- |
| Windows x64 后端 | PASS：本机真实 ViGEm spike；驱动正在运行，PnP 版本为 `1.21.442.0`、`oem15.inf`、已签名。官方 v1.22.0 release 明示 driver version unchanged，仍不能据此证明当前安装包来源 |
| Windows x64 native Flutter 产品构建/FFI/UI | BLOCKED：真实生成桥缺失；未实际渲染、点击、恢复/保存菜单或运行 FFI 路由测试 |
| ARM64 | 依赖投影 PASS；capability=false 仅为 cfg 静态核查；完整构建 BLOCKED（缺 ARM64 C 编译器） |
| 非 Windows 默认 workspace 测试 | BLOCKED：WSL Ubuntu 24.04 存在，但 login PATH 与用户 `.cargo` 均无 Linux Cargo；探测退出 127，未安装工具链或运行 Linux 测试 |
| 未安装驱动时启动/普通远控/DRIVER_MISSING | NOT RUN：当前驱动存在，未为测试卸载；仅 mock 初始化错误与映射测试通过 |
| 新旧端任意组合普通远控 | NOT RUN：仅能力缺失/false 的模型、I/O 门控和回环检查通过，未与旧版进程建立真实会话 |
| 两台 Windows、物理 XInput、实际游戏核心链路 | PASS（2026-11-11 后续阶段验收）：双端使用同一未签名便携构建，Brotato 可完全正常操控；证据边界见下节，未包含 `joy.cpl` 和故障/清理矩阵 |
| 双机故障与清理矩阵 | NOT RUN：热拔插、关闭、撤权、断网、退出、注销、服务重启、驱动故障和第二会话 `BUSY` 尚未逐项验收 |

后续人工验收仍须在 `joy.cpl` 逐项记录按钮、双扳机和四轴，并验证热拔插。保持输入时
分别执行关闭开关、撤销键鼠或 allow-gamepad、断网、退出、注销、服务重启和驱动故障，
检查归零及 target 消失；第二会话须收到 BUSY。另用无驱动机器验证 DRIVER_MISSING、
用新旧端双向组合验证普通远控。这些剩余项目当前均未执行，不承诺强杀后的驱动清理。

### 阶段性里程碑验收：核心端到端链路（2026-11-11）

本阶段使用两台 Windows x64，控制端连接物理 XInput 手柄，被控端使用 ViGEmBus；两端
运行同一未签名便携构建 `1.5.1+69`，对应提交
`a62443701aff3b7fe57e8d482e54c4a28189faa4`。RustDesk 手柄菜单实际显示
`Local: Connected` 和 `Remote: Gamepad is active`，证明控制端采集、协议协商、远端
授权及 ViGEm 目标均处于活动状态。

用户在远端实际 XInput 游戏 Brotato 中确认可以完全正常操控。针对此前疑似“连点”的
现象，电脑控制先记录主菜单选中“开始”，随后用户仅短按一次十字键“下”；复查画面时
选中项只移动到相邻的“物品套组”，没有跳过多个项目，也未复现重复输入。由此将
**单手柄正常交互的核心端到端链路**记为阶段性 PASS。

该结论不扩展到 `joy.cpl` 全按钮/扳机/轴验证、热拔插、关闭、撤权、断网、退出、注销、
服务重启、驱动故障、第二会话 `BUSY`、新旧端组合或强杀清理；这些项目继续保留为后续
发布验收门禁。

### 来源、许可证与增量 SBOM

本次再次从[官方 v1.22.0 release](https://github.com/nefarius/ViGEmBus/releases/tag/v1.22.0)
下载原资产，只保存到系统临时目录，未执行。release API 发布时刻为
`2023-11-02T16:40:49Z`；大小、SHA-256 和 Authenticode Valid 结果均与 Task 2 一致。
[官方 LICENSE](https://github.com/nefarius/ViGEmBus/blob/v1.22.0/LICENSE) 为 BSD-3-Clause，
要求源码/二进制再分发保留相应声明且不得借作者名背书。
[官方 EOL 声明](https://docs.nefarius.at/projects/ViGEm/End-of-Life/)明确 ViGEmBus 及其
官方客户端不再更新；仓库于 2023-11-02 归档。MVP 接受外部 EOL 驱动依赖的风险，
限制为显式启用、精确 crate、可替换后端、不捆绑驱动；不将此视为长期维护保证。

下表是本功能 Windows MSVC x64 的增量组件清单，**不是完整 RustDesk 发布 SBOM**。
实际 `cargo tree -p vigem-client --edges normal --target x86_64-pc-windows-msvc` 仅为
`vigem-client 0.1.4 -> winapi 0.3.9`；`winapi` 是原有依赖，未新增其他包或启用 unstable
feature。发布前仍须把这些项目及许可证全文纳入完整发布 SBOM/第三方通知。

| 组件 | 来源与许可证 | SHA-256 / 产物边界 |
| --- | --- | --- |
| `pkg:cargo/vigem-client@0.1.4` | [精确源码 Cargo.toml](https://github.com/CasualX/vigem-client/blob/acbc38bbc159e315622b5dbd9b41048fc9f3164c/Cargo.toml)、[MIT 许可](https://github.com/CasualX/vigem-client/blob/acbc38bbc159e315622b5dbd9b41048fc9f3164c/license.txt)；[registry API](https://crates.io/api/v1/crates/vigem-client/0.1.4) 记录发布于 `2022-08-01T19:42:48.076083Z`，未 yanked | `b857e6f99efe1e1eb1e4dfb035de8ae7ec8ec56bd1928edcbd7c6e4427634d52`；本次重新下载与 registry/lockfile 一致，编入 Windows x64 |
| `pkg:cargo/winapi@0.3.9` | [上游 manifest](https://github.com/retep998/winapi-rs/blob/0.3.9/Cargo.toml) 为 MIT OR Apache-2.0；[MIT](https://github.com/retep998/winapi-rs/blob/0.3.9/LICENSE-MIT)、[Apache](https://github.com/retep998/winapi-rs/blob/0.3.9/LICENSE-APACHE) | `5c839a674fcd7a98952e593242ea400abe93992746761e38641405d28b00f419`；本地发布包实测与 lockfile 相同，是既有依赖 |
| ViGEmBus setup `v1.22.0` | 官方 release 及 BSD-3-Clause，见上 | `89220a7865076b342892f98865f3499fb7c4cfd673159e89d352c360fd014c6a`；外部前置条件，不进入 RustDesk 提交或本地 target 产物 |

本次 [CasualX 上游仓库](https://github.com/CasualX/vigem-client) API 为 `archived=false`，
最后 push 时间为 `2023-08-11T10:32:02Z`；没有查到该作者的 EOL 声明。故修正原先
“crate 已停止维护”的无充分来源措辞，保留长期无更新与驱动 EOL 的供应链风险。
不能把官方客户端项目的 EOL 直接当作第三方 Rust crate 的独立声明。

### 最小化裁决与已知限制

- 删除 Task 7 在 `en.rs` 添加的 10 条英文等于 key 的空项；检查先 RED（退出 1）、
  删除后 GREEN（退出 0）。英文继续回退到 key，其他语言和既有翻译保持不变。
- 不将 `gamepad-passthrough` 加入 KEYS_*：实际 peer 保存直接写 options；这些列表
  唯一消费者是 custom-client default/override 分类，新增分类会扩大行为范围。
- Task 4 的完整正向 `on_message` enable/input/disable mock 集成测试仍未补：生产
  enable 固定真实平台工厂，既有 fixture 只能独立注入 helper；不为测试增加生产 hook
  或模拟桥。现有实际拒绝入口、会话/权限清理测试和 client 回环不替代此覆盖。
- 保留已审查限制：重复 enable 清理期间可能短暂 BUSY；全局撤销后旧 session 的单独
  disable 可能无第二次回调；同一网络连接新窗口在下次低频变化前没有状态重放；收到
  DISABLED_BY_REMOTE 后远端重新授权需要重连/重新取得状态。底层后端若永久阻塞，
  owner 保留到清理完成；不增加强夺所有权的状态或超时机制。
- 没有新 Cargo feature；纯逻辑测试由默认测试编译，Windows x64 后端和 Flutter FFI
  由现有 Windows release feature 测试覆盖，不修改 CI 工作流。CI 未在本任务实际运行。

最终逐文件回归面检查确认：依赖文件仅增 Windows x64 依赖；protobuf 仅追加字段；
config keys 仅注册显式授权；server 的连接/IPC 只增加手柄分派与撤权/关闭 hook；
client 的 I/O 只增加可选采集、状态和结束清理（失联 KCP 仍使用原 500ms 期限）；
Session/Flutter 只加低频能力、启停和事件；桌面 toolbar/settings、Web no-op bridge、
语言表是必要功能入口。既有 Features 定义移至实际产品模型并 re-export，隐私能力
语义不变。键鼠/媒体旧实现未改写，关闭/不支持时不启动采集或发送手柄消息。
`hbb_common`、移动端、Sciter、共享 toolbar、Web JS 和 portable-service 输入 IPC
均无分支差异。Task 8 产品修改仅上述英文表删除，其余为验证文档，未新增状态机、
计时器、缓存或无关修复。

### Task 8 修复轮 1：设置删除键时立即撤销

整体验收发现此前撤销 hook 误把 Options 当成局部补丁：普通客户端关闭开关发送
整张表并删除 `allow-gamepad`，旧 hook 对 None 不撤销；保持不变的输入可能继续
停留在后端，直到新输入或连接关闭才清理。此项是实际设置路径错误，不属于已接受
的旧 session 二次 disable 回调限制。

本轮删除更新前的可选值快照，成功保存后读取包含 default/override 的有效配置，
非 Y 即撤销；失败更新仍提前返回。缺键在撤销 helper 中也按关闭处理。其他单键
setter 先取得完整 options 再修改并发送；因此没有把局部 patch 的缺键当作关闭。
Task 4 计划同步修正整表语义，不修改 hbb_common 或通用配置 API。

现有 Connection/mock backend 清理测试改为先保留完整表中的 Y，再删除该键而不
发送下一条输入。旧实现出现等待 neutral 的 Timeout（退出 101）；修复后连接测试
4 passed、手柄专项 29 passed / 1 ignored、IPC 既有测试 3 passed（均退出 0）。此为
真实 session/worker 清理回归与配置语义静态核查，不是 native UI 点击或完整 IPC
持久化端到端验收；这些环境限制继续保留。本轮不增加状态、计时器或测试基础设施。
