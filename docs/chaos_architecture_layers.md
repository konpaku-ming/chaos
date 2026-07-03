# Chaos 仓库分层与模块说明

本文档是对 `chaos` 仓库的结构化阅读笔记。仓库里同时保留了两套“内核视角”：

- `kernel/src/kernel.rs`：当前作业与 `chaos-tests` 实际使用的单文件模拟内核。`chaos-tests/src/lib.rs` 是指向它的符号链接，因此测试导入的所有类型和函数都来自这里。
- `kernel/src/lib.rs` 及其子目录：原 rCore 风格的 `no_std` 多架构内核框架，包含架构初始化、驱动、系统调用、网络、信号、LKM、RVM 等模块。它解释了仓库的历史结构，但不是当前测试的主入口。

## 仓库层次

### L0：作业说明与文档层

- `README.md`：说明本仓库是基于 rCore 的调试/重写作业，核心目标是修复并重写 `kernel/src/kernel.rs`。
- `docs/`：阅读辅助文档目录。已有的 `kernel_annotated.rs` 是 `kernel.rs` 的中文注释版；本文档提供更高层次的模块划分。
- `ALLOCATOR_PROGRESS.md`、`MEMORY_ALLOCATION_LESSON.md`、`BUGFIX_LOG.md`：当前工作树中的补充记录文件，用来承载调试、分配器或修复日志。

### L1：测试层

- `chaos-tests/`：Rust 测试工程。它通过符号链接把 `kernel/src/kernel.rs` 当成库来测。
- `tests/basic`：覆盖锁、等待队列、引用计数、COW、进程生命周期、磁盘、挂载、环形缓冲、陷入控制、访问检查和基础工作流。
- `tests/advanced`、`tests/pressure`：更偏综合和压力场景。
- `tests/allocator`：集中测试 `BuddyAllocator`、`HeuristicAllocator`、`FramePool`、`ZoneInfo`、`SharedPage`、`heap_grow` 等内存分配模块。

### L2：作业模拟内核层

- `kernel/src/kernel.rs`：一个在 `std` 环境下运行的“内核模拟器”。它用 `Arc`、`Mutex`、`RwLock`、`thread::park` 等标准库机制模拟内核中的锁、进程、文件、IPC、页帧池、系统调用和设备行为。
- 这个文件不是按 Rust `mod` 拆开的，而是按功能段落堆叠。阅读时应把“结构体 + impl + 相关自由函数”视作一个模块。

### L3：保留的 rCore 内核框架层

- `kernel/src/lib.rs`：`no_std` 内核根模块，声明 `arch`、`drivers`、`syscall`、`signal`、`lkm`、`net`、`rvm` 等模块，并定义 `kmain()` 事件循环。
- `kernel/src/arch/*`：按 ISA 拆分的启动、页表、陷入、中断、定时器、浮点上下文、系统调用号和板级初始化。
- `kernel/src/drivers/*`：串口、块设备、PCI/virtio MMIO、网卡、GPU、输入、RTC、IRQ、MMC 等驱动抽象与具体实现。
- `kernel/src/syscall/*`：真实 rCore 风格的异步 syscall dispatcher，按 `fs`、`proc`、`mem`、`net`、`ipc`、`signal`、`time` 等文件拆分。
- `kernel/src/signal`、`kernel/src/net`、`kernel/src/lkm`、`kernel/src/rvm`：分别对应信号递送、socket 网络栈、可加载内核模块、虚拟机支持。

### L4：启动、用户态、模块与工具层

- `rboot/`：x86_64 UEFI 启动器。它读取 `rboot.conf`，加载内核 ELF 和 initramfs，建立页表、物理内存映射和内核栈，然后把 `BootInfo` 传给内核入口。
- `user/`：用户态程序构建系统，包含 Rust 用户库、uCore/Biscuit/Alpine/libc 测试、nginx/redis/iperf 等移植应用。
- `user/rust/src/syscall.rs`：用户态 syscall 封装，按架构使用 `ecall`、`syscall`、`svc` 等指令进入内核。
- `modules/hello_rust/`：LKM 示例模块，导出 `init_module`，调用 `rcore::lkm::api::lkm_api_pong()`。
- `tools/`：镜像、调试、设备、GDB、OpenSBI、符号填充、Docker 等辅助脚本。
- `crate/memory/`：当前快照中只是满足 `kernel/Cargo.toml` 依赖的占位 crate，作业测试不使用它。

## `kernel/src/kernel.rs` 的分层

### K0：ABI 常量与全局约定

这一层定义模拟内核和测试共享的数值契约，包括页大小、内核地址边界、页帧数量、文件标志、ioctl 命令号、ELF auxiliary vector、VM 权限、capability、调度策略、信号号、timer wheel 参数、socket 类型和 syscall 编号。

代表内容：

- `PAGE_SZ`、`KERN_BASE`、`PHYS_OFF`、`MEM_OFF`：地址空间和页帧换算的基础。
- `SYS_*`：`Kernel::dispatch_syscall` 使用的系统调用编号。
- `VM_*`、`CAP_*`、`SIG*`、`SCHED_*`：虚拟内存、权限、信号、调度模块共用的位标志。

这一层不做行为，只提供所有子系统的共同语言。

### K1：同步、事件与等待

这一层模拟内核里最基础的并发控制。

- `KernLock` / `GKL`：全局内核锁。内部记录 `flag`、`holder`、递归 `depth` 和线程本地 `thread_id`，支持同线程重入、`try_enter`、`held`、`owner`、`level`。`Kernel::tick` 和部分缓存同步逻辑会直接使用它。
- `Spin`：简单自旋锁，包装 `AtomicBool`，提供 `acquire`、`try_acquire`、`release`、`is_held`。`BlockCache`、`Channel` 等短临界区使用它。
- `FlgGuard`：占位式 guard，接口上像“关中断/恢复中断”的 RAII 对象，但当前实现没有真实副作用。
- `EvFlag` / `EvBus` / `wait_ev`：事件位图与回调总线。`EvFlag` 定义 READABLE、WRITABLE、CLOSED、PROC_QUIT、CHILD_QUIT、RECV_SIG、SEM_RM、SEM_ACQ 等事件；`EvBus::change` 修改事件并触发一次性回调；`wait_ev` 轮询等待某些事件出现。
- `SyncQueue`：用 `thread::park/unpark` 模拟睡眠队列，支持单唤醒、广播、唤醒 N 个、事件等待、超时等待和 epoll 注册记录。
- `Sema` / `SemaGuard`：计数信号量。`try_acquire` 检查删除状态和计数，`access` 返回 RAII guard，guard drop 时自动 `release`。
- `FutexBucket` / `FutexTable`：按用户地址组织的 futex 等待和唤醒结构。`FutexBucket` 是较完整版本，支持 wait、wake、requeue、pending 统计；`FutexTable` 是更简化的全表队列。
- `WaitQueue`：后置通用等待队列，按 `key` 和 `flags` 管理等待线程，提供 `sleep`、`sleep_timeout`、`wake_one`、`wake_all`、`wake_filtered`、优先级重排和统计。

这一层被 IPC、管道、任务退出、信号、futex syscall 和设备模拟复用。

### K2：地址、页帧与虚拟内存

这一层把“地址是否合法”“页帧如何分配”“虚拟区间如何维护”组合起来。

- `p2v`、`v2p`、`k_off`：物理地址、内核线性映射地址和内核偏移的换算工具。
- `PgFrame`：页帧引用计数对象，提供 `up`、`down`、`count`、`set`、`cas`、`inc_if_nonzero`，供 COW 和共享页测试使用。
- `VmRegion`：虚拟内存区域，包含 `base`、`len`、`flags`、`offset`、`tag`、`ref_count`。相关方法负责区间端点、包含关系、重叠判断、拆分、相邻合并和引用计数。
- `VmMap`：按地址排序维护多个 `VmRegion`，提供插入、二分查找、范围删除、寻找空洞、总映射大小、clone 区域和 gap 计算。
- `ZoneInfo`：模拟 DMA/NORMAL/HIGH 等内存 zone，维护 PFN 范围、空闲计数、水位线和压力计算。
- `BuddyAllocator`：经典 buddy 分配器。按 order 保存 free list，`alloc_order_aligned` 从合适 order 中寻找满足对齐的块，拆分大块；`free` 按 `addr_order_map` 找回真实 order 并向上合并 buddy；同时维护统计信息和碎片率。
- `HeuristicAllocator`：在 buddy 基础上加入保留策略和反馈策略。它会为低阶 order 保留目标块数，保护高阶块，在压力下主动 coalesce，并根据近期分配请求动态调整 target。
- `FramePool`：页帧池门面。内部持有 `Mutex<HeuristicAllocator>`，对外给出 frame id 级别的 `get_inner`、`get_contig`、`put`、`avail`、`free_count`、zone-aware 分配和 batch 分配。
- `frame_alloc`、`frame_dealloc`、`frame_alloc_contig`：把 `FramePool` 的 frame id 转成 `MEM_OFF + frame * PAGE_SZ` 形式的地址接口。
- `SharedPage`：COW 模拟对象。第一次 `fault` 会从 `FramePool` 分配新帧、降低源 `PgFrame` 引用计数，并把页面标记为可写且不再 pending。
- `KStk`：用 boxed slice 模拟内核栈，`top` 返回栈顶，`Drop` 中释放原始内存。
- `check_access`、`check_access_rw`、`cfu`、`ctu`、`validate_access`：用户地址访问检查。重点是拒绝越过 `KERN_BASE`、处理长度为零、检测整数溢出和页跨度。
- `heap_init`、`heap_grow`：堆初始化与扩容模拟；`heap_grow` 从 `FramePool` 连续拿页，并尝试把相邻虚拟段合并。
- `AddrSpace`：更完整的地址空间对象，组合 `VmMap`、页表根、ASID、引用计数和 COW 页表。支持 fork 复制区域、COW fault、unmap、protect、RSS 和共享页统计。

### K3：字节缓冲、终端与通道

- `CircBuf`：固定容量环形缓冲，维护 `rd`、`wr`、`cap`、`n`，提供 `push`、`pop`、`peek`、`drain_to`、`fill_from`、容量查询。基础测试的 ring buffer 主要看它。
- `Channel`：带 `CircBuf`、`Spin` 和 `SyncQueue` 的生产者/消费者通道。`recv` 在无数据时 park，`send` 写入后唤醒等待者，`close` 唤醒所有线程，另有 `try_recv`、`send_batch`、`drain_all` 和容量查询。
- `TrmIO` / `WinSz`：终端 ioctl 使用的数据结构，模拟 Linux termios 和窗口大小。
- `ser`、`tty_push`、`tty_pop`：把回车规范化为换行，并维护 `Kernel` 内部的 TTY 输入缓冲。

### K4：文件描述符、文件、管道与 epoll

- `FdOpt` / `FdState`：文件描述状态。`FdOpt` 表示读、写、追加、非阻塞；`FdState` 保存 offset、选项和锁状态，并被 `Arc<RwLock<_>>` 共享。
- `FHandle`：普通文件模拟。它把路径、文件内容 `Vec<u8>`、描述符状态、pipe 标志和 close-on-exec 标志组合起来。方法覆盖 `read`、`read_at`、`write`、`write_at`、`seek`、`set_len`、`metadata_sz`、`fallocate`、`splice_to`、`mmap`、`io_ctl` 等。
- `PipeNode` / `PipeBuf` / `PipeDir`：管道读写端。`pair` 创建共享缓冲的读端和写端；读空且两端都在时返回 `again`；写入后设置 READABLE；drop 任一端会减少 `ends` 并发 CLOSED 事件。
- `FLike`：文件描述符表中的统一枚举，可能是 `File`、`Pipe` 或 `Ep`。它统一实现 `dup`、`read`、`write`、`io_ctl`、`mmap_fl`、`poll`，让 `Task.files` 不需要关心底层类型。
- `PseudoNode`：只读伪文件节点，支持 `read_at` 和元数据大小查询。
- `EpData` / `EpEvent` / `EpCtlOp` / `EpInst`：epoll 模拟。`EpEvent` 保存事件 mask 和用户数据，`EpInst` 维护 fd 到事件的映射，以及 ready/new_ctl 集合；`control` 支持 ADD、MOD、DEL。
- `audit_fd_table`：扫描 fd 表中的空洞、异常 pipe 和空路径文件，用于测试或诊断。

### K5：缓存、对象注册、挂载、I/O 调度与磁盘

- `PageCache` / `PageCacheEntry`：页缓存，按 page id 保存数据、dirty 位、访问 tick 和 pin 计数。支持 lookup 命中统计、插入、LRU 淘汰、dirty 标记、writeback、pin/unpin、invalidate 和范围 flush。
- `KObjRegistry` / `KObjEntry`：内核对象注册表。按 id 记录类型、owner、创建 tick、引用计数和父子关系，并维护类型索引；支持注册、注销、按类型查找、对象图 dump、GC sweep 和 owner 查询。
- `BlockCache` / `CacheChain` / `CacheSlot`：哈希链式块缓存。每条链有 `Spin` 和 `Mutex<Vec<CacheSlot>>`；`fetch` 命中则复制 payload，未命中则模拟延迟并生成 512 字节块；还支持全量同步、失效、统计和冷块淘汰。
- `MountTable` / `MountEntry`：挂载表。`bind` 增加 prefix 到 target 的映射并按 prefix 长度降序排列；`resolve` 选择最长前缀匹配并递归解析；还支持 unmount、list、find、count 和 prefix 检查。
- `IoQueue` / `IoRequest`：块 I/O 调度队列。`dispatch` 根据磁头位置和方向模拟 elevator/SCAN，`merge_adjacent` 合并相邻且方向相同的请求。
- `Disk`：块设备模拟。`errs` 表示剩余错误次数，`usize::MAX` 表示持续失败；`read_block` 和 `read_block_n` 会重试并可经 journal 设备递归读取；`write_block`、`flush` 维护操作计数和错误行为。

### K6：System V IPC 与共享内存

- `IpcPerm` / `SemDs`：System V semaphore 的权限和描述结构。
- `SemArr`：一组 `Sema`。`get_or_create` 通过全局 weak store 按 key 复用或创建 semaphore array；`set_ds` 只允许更新 uid/gid/mode 等元数据。
- `SemCtx`：进程私有的 semaphore 上下文。保存 semid 到 `SemArr` 的映射和 undo 记录；clone 时继承数组但不继承 undo；drop 时按 undo 释放信号量。
- `ShmTag` / `ShmCtx` / `shm_get_or_create`：共享内存上下文。全局 weak store 按 key 复用页数组，进程内 `ShmCtx` 管理 shmid 到地址与页集合的映射。

### K7：进程、线程、调度与进程组

- `ProcInit`：构造用户栈初始布局的模拟器。`push_at` 根据 args、envs、auxv 计算新栈顶，`total_size` 估算所需空间。
- `Context`：通用寄存器上下文。保存 `N_REGS` 个寄存器、`ip` 和 `flags`，支持 capture/apply、设置 IP/SP/返回值/TLS、系统调用参数提取、diff 和 hash。
- `TrapCtl`：陷入/中断控制器模拟。维护硬件/软件 mask、嵌套深度、当前 frame、frame stack、IRQ 开关和 suppress 状态；支持 dispatch、IRQ 处理、page fault 检查和按 vector 分发。
- `SchedulePolicy`：调度策略、优先级、nice、时间片和 vruntime。`weight` 用近似 Linux CFS 的 nice 权重。
- `RunQueue`：运行队列。保存 `(task_id, policy)`，支持 enqueue 排序、dequeue 选择、pick_next、rebalance、current 设置、preempt_disable/enable、priority boost 和 yield。
- `Pid` / `TaskInfo` / `ThdCtx`：进程号、可展示任务信息和线程上下文。
- `Task`：模拟进程/线程实体，是文件、cwd、exec 路径、futex、semaphore、shared memory、pid/pgid、线程列表、事件总线、退出码、信号队列、epoll、内核栈和用户上下文的聚合点。方法覆盖父子链接、fd 管理、退出、信号、epoll、线程上下文切换、dup/dup2/close 等。
- `TaskTable`：全局任务表。负责 spawn、spawn_root、find、按 tag/线程/pgid 查找、注册 pid、reap、fork_task、clone_thread、new_user_task、发送进程组信号、列出 active/zombie。
- `ProcessGroup`：进程组和会话模拟。维护成员、leader、session、前台状态，并能向组内成员广播信号。
- `ResourceLimits`：资源限制对象，覆盖 fd、线程、栈、数据段、文件大小、映射数量和 CPU 时间，可继承、设置、查询和检查是否超限。

### K8：权限、信号与时间

- `CapSet`：Linux capability 模拟。维护 permitted/effective/ambient 三类位图，支持检查、授予、丢弃、继承、ambient 提升和清空。
- `SigAction` / `SigSet`：信号动作和 pending/blocked 集合。支持 raise、clear、block/unblock/setmask、选择 deliverable 信号、设置动作、查询忽略和 exec 后清理非默认 handler。
- `TimerEntry` / `TimerWheel`：定时器和时间轮。entry 记录 deadline、interval、callback id、active/repeat；wheel 按 deadline 分桶，`advance` 返回到期项并重排周期定时器。
- `CLK` / `CLK_ALL` / `wclk` / `cclk` / `dtk` / `tmr` / `up_ms`：全局 tick 计数。`dtk` 只让 CPU0 推进 `CLK`，所有 CPU 都推进 `CLK_ALL`。

### K9：网络与协议工具

- `SocketState`：TCP socket 状态枚举，从 Closed 到 TimeWait、LastAck、Closing 等。
- `tcp_checksum`、`compute_inet_checksum`：Internet checksum 计算，前者附带 TCP pseudo-header 的源/目的地址和协议号。
- `parse_ipv4_header`：解析 IPv4 包头，检查版本、IHL、长度并返回源地址、目的地址、协议号和总长度。
- `build_pseudo_header`：构造 TCP/UDP checksum 所需的 12 字节 pseudo-header。

这一层在 `kernel.rs` 中是工具函数级别；真正的 rCore 网络栈在 `kernel/src/net/structs.rs`，那里用 smoltcp 管理 TCP/UDP/raw/packet/netlink socket 状态。

### K10：系统调用与 Kernel 门面

- `Kernel`：模拟内核顶层对象，聚合 `TaskTable`、`BlockCache`、`FramePool`、CPU 当前任务数组、`MountTable`、semaphore/shared-memory weak store、TTY 缓冲和 `Disk`。
- 生命周期方法：`new` 初始化子系统；`proc_init` 创建 init；`tick` 模拟时钟下的缓存同步；`schedule_tick`、`balance_load` 做调度 tick 和负载均衡；`reclaim_zombies` 清理 zombie。
- 任务方法：`cur_task`、`set_cur`、`spawn_thread`、`do_fork`、`do_exec`、`do_pipe`、`do_wait`。
- 内存和路径方法：`handle_pgfault`、`handle_pgfault_ext`、`alloc_pages`、`free_pages`、`memory_pressure`、`lookup_path`。
- IPC 方法：`get_sem`、`get_shm`。
- `dispatch_syscall`：按 `SYS_*` 分发系统调用。覆盖读写、open/close/stat、mmap/munmap/brk、ioctl、pipe、dup/dup2、fork/exec/exit/wait4、kill、fcntl、getpid/getppid、setpgid/getpgid/setsid、epoll、clock_gettime、sigaction、sigprocmask、futex。它更像“测试用 ABI 适配层”，主要做参数检查、地址合法性检查、状态更新和返回值模拟，而不是真正执行用户内存拷贝。

### K11：通用算法和诊断函数

- `validate_elf_header`：检查 ELF magic、class、endianness、版本、类型、program header 范围和 LOAD 段数量，返回入口地址。
- `compute_load_balance`：根据每 CPU 任务数、优先级、I/O 阻塞状态计算迁移目标 CPU。
- `rehash_mount_cache`、`defragment_frame_pool`、`verify_page_alignment`、`compute_rss_watermark`：挂载缓存、页帧碎片、页对齐和 RSS 水位诊断。
- `mem_scan_pattern`：KMP 模式匹配。
- `compute_crc32`、`encode_varint`、`decode_varint`：校验和与可变长整数编码工具。
- `bitwise_merge`、`rotate_bits`、`popcount64`、`clz64`、`ffs64`、`align_up`、`align_down`、`is_power_of_two`、`log2_floor`、`order_for_pages`、`hash_combine`、`murmurhash3_finalize`：底层位运算、对齐、order 计算和哈希辅助函数。

## 保留 rCore 框架的模块说明

### `kernel/src/lib.rs`

这是真实内核 crate 的根。它启用 nightly feature、声明 `no_std`、导入 alloc/log/lazy_static 等依赖，重新导出 `LockedHeapWithRescue`，声明内核模块，并定义：

- `kmain()`：循环运行 executor，空闲时调用 `arch::interrupt::wait_for_interrupt()`。
- `HEAP_ALLOCATOR`：全局堆分配器，通过 `memory::enlarge_heap` 扩容。

### `kernel/src/arch/*`

ISA 相关层的责任是把硬件差异隔离在 `arch` 下：

- `consts`：架构名、内核偏移和地址布局。
- `cpu`：CPU id、本地 CPU 初始化。
- `timer`：时钟初始化和当前时间。
- `interrupt`：中断/异常识别、ack、timer interrupt、page fault、trap handler。
- `paging`、`memory`：页表项、页表切换、物理内存初始化和地址转换。
- `syscall`：各架构 syscall 编号。
- `fp`、`signal`：浮点状态和信号上下文。
- `board/*`：平台相关初始化，例如 x86 PC、RISC-V virt/u540、Raspberry Pi 3、MIPS Malta。

### `kernel/src/drivers/*`

驱动层提供统一 trait 和分类注册表：

- `Driver`：所有驱动的公共 trait，包括中断处理、设备类型、id 和向 net/block/rtc trait 的 downcast。
- `DRIVERS`、`NET_DRIVERS`、`BLK_DRIVERS`、`RTC_DRIVERS`、`SERIAL_DRIVERS`：全局驱动列表。
- `BlockDriverWrapper`：把内核 block driver 包装成 `rcore_fs::dev::BlockDevice`。
- 子目录按设备类型拆分：block、bus、console、device_tree、gpu、input、irq、mmc、net、provider、rtc、serial。

### `kernel/src/syscall/*`

真实 syscall 层以 `handle_syscall(thread, context)` 为入口：

- 从架构 trapframe 中取 syscall 号和 6 个参数。
- 构造 `Syscall` 上下文，提供当前进程和虚拟内存访问方法。
- `syscall` match 按 syscall 编号分发到 `fs`、`proc`、`mem`、`net`、`ipc`、`signal`、`time`、`misc`、`custom` 等文件。
- `SysError` 统一把内核错误映射为 syscall 返回错误。

这套代码体现 rCore 的真实设计；`kernel.rs` 则是为了作业测试把这些思想压缩成一个可在普通 Rust 测试里运行的模拟版本。

### `kernel/src/signal`

信号层包含：

- `Signal` 枚举：Linux 常见信号号。
- `Sigset`、`SignalAction`、`Siginfo`：信号 mask、handler 行为和 siginfo 数据。
- `send_signal`：向进程/线程投递信号。
- `handle_signal`：在返回用户态前构造 signal frame，切换到用户 handler。
- `SignalUserContext`、`SignalFrame`、`SignalStack`：用户态信号栈和恢复上下文。

### `kernel/src/net`

网络层以 `Socket` trait 为统一接口，并提供多类 socket 状态：

- `TcpSocketState`、`UdpSocketState`：基于 smoltcp socket handle 的 TCP/UDP。
- `RawSocketState`、`PacketSocketState`：原始包和链路层包。
- `NetlinkSocketState`：模拟 netlink 消息、接口和路由信息。
- `Endpoint`：链路层或 netlink endpoint。

### `kernel/src/lkm`

LKM 层支持可加载内核模块：

- `ModuleInfo`：解析模块元信息，包括名称、版本、API 版本、导出符号和依赖。
- `LoadedModule`：保存模块元信息、导出符号、引用计数、虚拟地址空间、锁和加载状态。
- `ModuleState`：Ready、PrepareUnload、Unloading。
- `KObject`：类似 Linux kobject，用于和模块引用计数关联。
- `api`、`kernelvm`、`manager`、`const_reloc`：分别负责导出 API、模块虚拟地址空间、模块管理和不同架构的常量重定位。

### `kernel/src/rvm`

RVM 层在启用 `hypervisor` feature 时使用：

- `Guest`：包装 RVM guest，并把 guest physical memory 映射到当前进程虚拟地址空间。
- `Vcpu`：包装 RVM vCPU，不同架构有不同中断状态。
- `rvm_extern_fn`：给 RVM crate 提供宿主内核函数，例如连续页分配、释放、物理转虚拟和架构 trap hook。
- `RvmINode`：把虚拟机控制暴露成文件系统节点。

## 主要数据流

### 测试目标数据流

1. `chaos-tests` 导入 `kernel/src/kernel.rs`。
2. 测试直接构造 `Kernel`、`TaskTable`、`FramePool`、`BuddyAllocator`、`HeuristicAllocator`、`MountTable`、`Disk` 等对象。
3. 综合测试通常走 `Kernel::new` -> `proc_init` / `set_cur` -> `dispatch_syscall` 或 `do_*` 方法。
4. `dispatch_syscall` 再调用访问检查、任务表、文件描述符、页帧池、缓存、挂载、信号和 futex 等子模块。

### 内存分配数据流

1. `FramePool::new(n)` 创建 `HeuristicAllocator`，地址基准是 `MEM_OFF`。
2. 单页分配走 `FramePool::get_inner` -> `HeuristicAllocator::alloc_page` -> `alloc_order(0)`。
3. 连续分配走 `get_contig` -> `alloc_order_aligned(order, align)`。
4. 释放走 `FramePool::put` -> `HeuristicAllocator::free` -> `free_to_buddy`，可能触发 buddy 合并或保留策略。
5. `frame_alloc` / `frame_dealloc` 负责把 frame id 接口转换成地址接口。

### 进程与文件数据流

1. `TaskTable::new_user_task` 创建 `Task`，设置 exec path、用户栈上下文和标准输入输出错误。
2. `Task.files` 保存 `BTreeMap<fd, FLike>`。
3. `FLike` 把普通文件、pipe、epoll 实例统一成 read/write/poll/ioctl/mmap 接口。
4. `fork_task` 复制 cwd、exec path、fd 表、pgid、semaphore/shared memory 上下文和信号 mask。
5. `exit_proc` 关闭 fd、设置退出事件、通知父进程、记录退出码并清空线程列表。

### rCore 启动数据流

1. `rboot` 在 UEFI 中读取配置、加载内核 ELF、建立页表、构造 `BootInfo`。
2. 架构入口如 x86_64 `_start` 或 RISC-V `rust_main` 初始化日志、堆、内存、trap、定时器、驱动、进程。
3. 初始化完成后进入 `kmain()`，不断运行 executor，并在空闲时等待中断。

## 阅读建议

- 如果目的是通过当前作业测试，优先读 `kernel/src/kernel.rs` 的 K1 到 K11，尤其是分配器、FramePool、Task/TaskTable、Kernel::dispatch_syscall。
- 如果目的是理解原 rCore 设计，先读 `kernel/spec.md`、`kernel/src/lib.rs`、对应架构的 `arch/*/mod.rs`，再读 `drivers` 和 `syscall`。
- 如果某个测试失败，先看测试文件名：`allocator` 基本对应 K2，`group_01/02/03` 对应 K1，`group_04` 对应页帧引用和 COW，`group_05/10/11` 对应任务/Kernel 工作流，`group_06/07/08/09` 分别对应 Disk、MountTable、CircBuf、TrapCtl。
