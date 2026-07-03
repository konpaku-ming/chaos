# `kernel/src/kernel.rs` 分层结构体与方法清单

本文档参考 [chaos_architecture_layers.md](./chaos_architecture_layers.md) 中“`kernel/src/kernel.rs` 的分层”部分，对 `kernel/src/kernel.rs` 里的结构体、枚举、主要自由函数做逐层归档。重点是说明每个结构体的字段含义、方法职责，以及它在模拟内核中的角色。

说明：

- “字段”只列实际定义在结构体里的字段；对于 tuple struct 和零字段标记类型，会单独说明。
- “方法”包含 `impl Type` 中的方法，也会注明关键 trait impl，例如 `Drop`、`Default`、`Index`、`Display`。
- 有些类型在原分层文档里没有点名，例如 `SlabEntry`、`BuddyStatistics`、`HeuristicPolicy`，但它们确实属于 `kernel.rs` 的某个子系统，因此也放入对应层。
- 枚举不是结构体，但它们定义了同层 API 的重要值域，本文也放在“相关枚举/标记类型”里。

## K0：ABI 常量与全局约定

这一层主要是常量、位标志、系统调用号和全局类型别名，没有独立结构体方法。它给后续层提供共同的数值语言。

### 常量类别

- 地址与容量：`PAGE_SZ`、`N_PROC`、`N_FRAMES`、`KERN_BASE`、`PHYS_OFF`、`MEM_OFF`、`KHEAP_SZ`、`KSTK_SZ`、`USR_STK_OFF`、`USR_STK_SZ` 等。
  - 用于页大小、内核地址边界、物理/虚拟偏移、内核堆、内核栈、用户栈等模拟地址空间约定。
- 文件与 fd 标志：`F_DUPFD`、`F_GETFD`、`F_SETFD`、`O_NONBLOCK`、`O_APPEND`、`O_CLOEXEC`、`FD_CLOEXEC` 等。
  - 被 `FHandle`、`FLike`、`Kernel::dispatch_syscall` 的 `fcntl/open/pipe/ioctl` 分支复用。
- ioctl 与终端：`TCGETS`、`TCSETS`、`TIOCGPGRP`、`TIOCSPGRP`、`TIOCGWINSZ`、`FIONBIO` 等。
  - 配合 `TrmIO`、`WinSz` 和 `SYS_IOCTL` 分支做参数检查。
- ELF auxiliary vector：`AT_PHDR`、`AT_PHENT`、`AT_PHNUM`、`AT_PAGESZ`、`AT_BASE`、`AT_ENTRY`。
  - 配合 `ProcInit` 计算用户栈初始布局。
- 虚拟内存标志：`VM_READ`、`VM_WRITE`、`VM_EXEC`、`VM_SHARED`、`VM_GROWSDOWN`、`VM_DONTCOPY`、`VM_HUGETLB`、`VM_PFNMAP`。
  - 被 `VmRegion`、`AddrSpace`、`SYS_MMAP` 等使用。
- capability 常量：`CAP_CHOWN`、`CAP_KILL`、`CAP_SETUID`、`CAP_SETGID`、`CAP_NET_BIND`、`CAP_NET_RAW`、`CAP_SYS_ADMIN`、`CAP_SYS_PTRACE`、`INHERITABLE_MASK`。
  - 被 `CapSet` 用来检查、授予和继承权限位。
- 内存 zone：`ZONE_DMA`、`ZONE_NORMAL`、`ZONE_HIGH`、`N_ZONES`。
  - 与 `ZoneInfo` 的 zone-aware 页帧分配相关。
- 调度：`PRIO_MIN`、`PRIO_MAX`、`PRIO_DEFAULT`、`SCHED_NORMAL`、`SCHED_FIFO`、`SCHED_RR`、`SCHED_BATCH`。
  - 被 `SchedulePolicy`、`RunQueue` 使用。
- slab：`SLAB_OBJ_MIN`、`SLAB_OBJ_MAX`、`SLAB_ALIGN`。
  - `SlabEntry` 以 `SLAB_ALIGN` 对齐对象大小。
- 信号：`NSIG`、`SIG_DFL`、`SIG_IGN`、`SIGKILL`、`SIGSTOP`、`SIGCHLD`、`SIGUSR1`、`SIGUSR2`、`SIGALRM`。
  - 被 `SigSet`、`Task::send_sig`、`Kernel::dispatch_syscall` 使用。
- 时间轮：`TIMER_WHEEL_SIZE`、`TIMER_TICK_HZ`、`USEC_TICK`、`BOOT_EPOCH`。
  - 被 `TimerEntry`、`TimerWheel`、`wclk/up_ms` 和 `clock_gettime` 模拟使用。
- socket 与 syscall：`SOCK_STREAM`、`AF_INET`、`SYS_READ`、`SYS_WRITE`、`SYS_OPEN`、`SYS_FORK`、`SYS_FUTEX` 等。
  - `SocketState` 与 `Kernel::dispatch_syscall` 的 ABI 基础。

## K1：同步、事件与等待

### `KernLock`

职责：全局内核锁 `GKL` 的实际类型。它用原子变量模拟可重入的大内核锁，同一宿主线程重复进入时只增加递归深度。

字段：

- `flag: AtomicBool`：锁是否已被占用，`true` 表示已有持有者。
- `holder: AtomicUsize`：调用者传入的逻辑 owner id，主要用于诊断和测试。
- `depth: AtomicUsize`：同一线程重入深度；第一次进入为 1，重入时递增。
- `thread_id: AtomicUsize`：线程本地分配的宿主线程 id，用来判断是否同线程重入。

方法：

- `new() -> Self`：构造未持有的锁，所有原子字段清零。
- `enter(id)`：阻塞式自旋进入锁；如果当前线程已持有，则只增加 `depth`。
- `leave()`：释放一次锁；重入深度大于 1 时只递减 `depth`，降到 1 时才真正清空 owner 并释放 `flag`。
- `held() -> bool`：读取 `flag`，判断当前是否有人持有锁。
- `owner() -> usize`：读取逻辑 owner id。
- `level() -> usize`：读取当前递归深度。
- `try_enter(id) -> bool`：非阻塞尝试进入；同线程重入直接成功，未占用时 CAS 成功，占用时返回 `false`。

Trait/静态对象：

- `unsafe impl Send/Sync`：允许作为全局静态锁跨线程共享。
- `GKL: KernLock`：全局内核锁实例。

### `Spin`

职责：轻量自旋锁，供短临界区使用，例如块缓存链和 `Channel`。

字段：

- `v: AtomicBool`：锁位，`true` 表示持有。

方法：

- `new() -> Self`：构造未持有的自旋锁。
- `acquire()`：循环 CAS，直到将 `false` 改成 `true`。
- `try_acquire() -> bool`：尝试 CAS 一次，成功返回 `true`。
- `release()`：直接将锁位写回 `false`。当前实现不校验调用者是否持有。
- `is_held() -> bool`：读取锁位。

Trait：

- `unsafe impl Send/Sync`：允许在多个线程之间共享。

### `FlgGuard`

职责：占位式 RAII guard，接口模拟“进入临界区/关中断”，但当前实现没有真实副作用。

字段：

- tuple 字段 `usize`：当前恒为 0，没有被行为逻辑使用。

方法：

- `enter() -> Self`：返回一个 guard。

Trait：

- `Drop::drop()`：空实现。guard 离开作用域时不会恢复任何真实状态。

### `EvFlag`

职责：事件位图的命名空间。它是零字段结构体，只承载关联常量。

字段：

- 无字段。

关联常量：

- `READABLE`：对象可读。
- `WRITABLE`：对象可写。
- `ERROR`：对象进入错误状态。
- `CLOSED`：对象关闭或对端关闭。
- `PROC_QUIT`：进程退出。
- `CHILD_QUIT`：子进程退出。
- `RECV_SIG`：收到信号。
- `SEM_RM`：信号量被删除。
- `SEM_ACQ`：信号量可获取。

### `EvBus`

职责：事件总线，保存当前事件位和一次性回调列表。

字段：

- `ev: u32`：当前事件位图。
- `cbs: Vec<Box<dyn Fn(u32) -> bool + Send>>`：事件变化时被调用的回调；返回 `true` 的回调会被移除。

方法：

- `make() -> Arc<Mutex<Self>>`：创建可共享、带互斥锁的事件总线。
- `set(s)`：设置 `s` 中的事件位。
- `clear(s)`：清除 `s` 中的事件位。
- `change(rst, s)`：先清除 `rst`，再设置 `s`；事件改变时触发回调清理。
- `sub(cb)`：注册一个回调。
- `cb_len() -> usize`：返回当前回调数量，常用于测试等待者计数。

相关类型/函数：

- `type EvCb = Box<dyn Fn(u32) -> bool + Send>`：事件回调类型别名。
- `wait_ev(bus, mask) -> u32`：轮询等待 `bus.ev & mask != 0`，满足后返回事件位图。

### `RegEp`

职责：记录某个等待队列注册到 epoll 的关系。

字段：

- `task_id: usize`：注册所属任务 id。
- `epfd: usize`：epoll 实例 fd。
- `fd: usize`：被监听的 fd。

方法：

- 无独立方法；被 `SyncQueue::reg_epoll` 和 `SyncQueue::unreg_epoll` 管理。

### `SyncQueue`

职责：通用同步等待队列，用 `thread::park/unpark` 模拟内核睡眠与唤醒。

字段：

- `q: Mutex<VecDeque<thread::Thread>>`：等待线程队列。
- `eq: Mutex<VecDeque<RegEp>>`：注册到 epoll 的等待记录。
- `pending_signals: AtomicUsize`：没有实际等待者时积累的信号数，避免一次 signal 丢失。

方法：

- `new() -> Self`：创建空等待队列。
- `park_on(g, pred) -> bool`：在互斥数据 `g` 上检查谓词；不满足则入队并 park，醒来后重新检查。
- `signal()`：唤醒一个等待线程；没有等待者时增加 `pending_signals`。
- `broadcast()`：唤醒所有等待线程。
- `signal_n(n) -> usize`：最多唤醒 `n` 个等待线程，返回实际唤醒数。
- `pending() -> usize`：返回等待线程数量。
- `wait_ev(g, cond) -> bool`：循环等待条件函数返回 `Some(result)`。
- `wait_events(queues, g, cond) -> bool`：同时把当前线程注册到多个队列，等待任一队列唤醒后重试条件。
- `wait_guard(g)`：入队、释放一次对 `g` 的锁获取结果并 park，接口上像等待某个 guard 条件。
- `wait_timeout(g, timeout) -> bool`：入队后带超时 park；当前实现总返回 `true`。
- `reg_epoll(task_id, epfd, fd)`：记录一个 epoll 注册项。
- `unreg_epoll(task_id, epfd, fd) -> bool`：删除匹配的 epoll 注册项。

### `SemaInner`

职责：`Sema` 的私有内部状态。

字段：

- `cnt: isize`：当前计数。
- `pid: usize`：最后操作者或拥有者 pid 记录。
- `rm: bool`：是否被删除。
- `bus: EvBus`：信号量状态事件总线，发布 `SEM_RM` 和 `SEM_ACQ`。

方法：

- 无独立 impl；由 `Sema` 通过 `Mutex<SemaInner>` 操作。

### `Sema`

职责：计数信号量。它属于同步层，也被 K6 的 System V semaphore array 复用。

字段：

- `inner: Arc<Mutex<SemaInner>>`：共享的信号量状态。

方法：

- `new(c) -> Self`：以初始计数 `c` 创建信号量。
- `remove()`：标记删除并发布 `SEM_RM`。
- `release()`：计数加一；若变为可获取，发布 `SEM_ACQ`。
- `try_acquire() -> Result<bool, &'static str>`：删除状态下返回错误；计数足够时减一并返回 `Ok(true)`，否则返回 `Ok(false)`。
- `acquire_spin() -> Result<(), &'static str>`：自旋/yield 直到获取成功或遇到删除错误。
- `access() -> Result<SemaGuard<'_>, &'static str>`：获取信号量并返回 RAII guard。
- `get_val() -> isize`：读取计数。
- `get_ncnt() -> usize`：返回事件总线回调数，近似等待者计数。
- `get_pid() -> usize`：读取 pid 字段。
- `set_pid(p)`：写入 pid 字段。
- `set_val(v)`：直接设置计数，计数大于等于 1 时发布 `SEM_ACQ`。

### `SemaGuard<'a>`

职责：信号量访问的 RAII guard。

字段：

- `s: &'a Sema`：被 guard 持有的信号量引用。

方法/Trait：

- `Drop::drop()`：guard 析构时自动调用 `Sema::release()`。
- `Deref<Target = Sema>`：允许像访问 `Sema` 一样访问 guard。

### `FutexBucket`

职责：按用户地址组织的 futex 等待桶，比 `FutexTable` 更完整，支持超时、唤醒和 requeue。

字段：

- `waiters: Mutex<VecDeque<(usize, thread::Thread, Arc<AtomicBool>)>>`：每个等待项包含地址、线程句柄和是否被显式唤醒的标志。

方法：

- `new() -> Self`：创建空 futex 桶。
- `wait(addr, expected, val, timeout) -> Result<(), &'static str>`：只有 `val == expected` 时入队；有超时则 `park_timeout`，否则 `park`；被 `wake` 标记后返回 `Ok(())`，否则返回 `"timeout"`。
- `wake(addr, count) -> usize`：唤醒指定地址上最多 `count` 个等待者。
- `requeue(src, dst, wake_n, move_n) -> usize`：先唤醒 `wake_n` 个 `src` 等待者，再把最多 `move_n` 个等待者迁移到 `dst`。
- `pending_at(addr) -> usize`：统计某地址上的等待者数量。

### `FutexTable`

职责：简化版全局 futex 表，按地址保存等待线程。

字段：

- `table: Mutex<VecDeque<(usize, thread::Thread)>>`：地址到等待线程的线性队列。

方法：

- `new() -> Self`：创建空表。
- `ftx_wait(addr, expected, val) -> bool`：若 `val != expected` 返回 `false`；否则入队并 park。
- `ftx_wake(addr, count) -> usize`：尝试唤醒指定地址上的等待线程，返回计数。
- `ftx_requeue(src_addr, dst_addr, wake_n, move_n) -> usize`：唤醒一批 `src_addr` 等待者，并把另一批改挂到 `dst_addr`。

### `WaitQueue`

职责：后置通用等待队列，按 key 和 flags 管理等待线程，支持过滤唤醒和优先级重排。

字段：

- `inner: Mutex<VecDeque<(usize, thread::Thread, u32)>>`：等待项，包含 key、线程句柄和 flags。
- `wake_count: AtomicUsize`：累计唤醒次数。

方法：

- `new() -> Self`：创建空等待队列。
- `sleep(key, flags)`：把当前线程按 key/flags 入队并 park。
- `sleep_timeout(key, flags, timeout) -> bool`：带超时睡眠；醒来后删除同 key 等待项，返回是否删除过。
- `wake_one(key) -> bool`：唤醒一个匹配 key 的等待者。
- `wake_all(key) -> usize`：唤醒所有匹配 key 的等待者。
- `wake_filtered(pred) -> usize`：用自定义谓词按 `(key, flags)` 过滤唤醒。
- `pending_count() -> usize`：返回等待项数量。
- `total_wakes() -> usize`：返回累计唤醒次数。
- `has_waiters_for(key) -> bool`：判断是否有指定 key 的等待者。
- `reorder_by_priority()`：按 flags 排序等待项，模拟优先级调整。

## K2：地址、页帧与虚拟内存

### `PgFrame`

职责：页帧引用计数对象，供 COW、共享页和地址空间模拟使用。

字段：

- `rc: AtomicUsize`：页帧引用计数。

方法：

- `new() -> Self`：创建引用计数为 0 的页帧。
- `with_rc(n) -> Self`：创建引用计数为 `n` 的页帧。
- `up() -> usize`：引用计数加一，返回增加前的值。
- `down() -> usize`：引用计数减一，返回减少前的值。
- `count() -> usize`：读取引用计数；代码中读取两次以规避瞬时变化。
- `set(n)`：直接设置引用计数。
- `cas(expected, desired) -> bool`：CAS 更新引用计数。
- `inc_if_nonzero() -> bool`：只有当前引用计数非零时才递增，用于避免复活空闲页。

### `VmRegion`

职责：虚拟内存区域，描述 `[base, base + len)` 的权限、偏移和引用计数。

字段：

- `base: usize`：区域起始虚拟地址。
- `len: usize`：区域长度。
- `flags: u32`：权限和属性位，例如 `VM_READ/VM_WRITE/VM_SHARED`。
- `offset: usize`：文件映射或区域内偏移。
- `tag: u16`：模拟用途的区域标签。
- `ref_count: AtomicUsize`：区域引用计数，常用于 fork/COW 测试。

方法：

- `new(base, len, flags) -> Self`：创建 offset 为 0、tag 为 0、引用计数为 1 的区域。
- `with_offset(base, len, flags, offset) -> Self`：创建带 offset 的区域。
- `end() -> usize`：返回末地址 `base + len`。
- `contains(addr) -> bool`：判断地址是否落在区域内。
- `overlaps(other) -> bool`：判断两个区域是否重叠。
- `split_at(addr) -> Option<(VmRegion, VmRegion)>`：在内部地址处拆成左右两个区域，同时调整右半段 offset。
- `merge_with(other) -> Option<VmRegion>`：相邻且 flags/tag 匹配时合并两个区域。
- `ref_up() -> usize`：引用计数加一。
- `ref_down() -> usize`：引用计数减一。
- `ref_get() -> usize`：读取引用计数。

### `VmMap`

职责：按地址排序维护多个 `VmRegion`，提供插入、查找、删除、空洞查找等基础 VMA 操作。

字段：

- `regions: Vec<VmRegion>`：虚拟内存区域列表，逻辑上按 `base` 排序。
- `brk: usize`：模拟进程堆顶，初始为 `0x0040_0000`。
- `mmap_base: usize`：匿名/文件映射搜索起点，初始为 `0x7000_0000`。

方法：

- `new() -> Self`：创建空映射表和默认 `brk/mmap_base`。
- `insert(region) -> Result<(), &'static str>`：按地址插入区域，若与已有区域重叠返回 `"overlap"`。
- `find(addr) -> Option<&VmRegion>`：二分查找包含地址的区域。
- `remove_range(base, len) -> usize`：删除与范围相交的区域，返回删除数量。
- `find_free(len, align) -> Option<usize>`：从 `mmap_base` 起寻找满足长度和对齐的空洞。
- `total_mapped() -> usize`：汇总所有区域长度。
- `clone_regions() -> Vec<VmRegion>`：深拷贝区域列表，同时复制引用计数当前值。
- `gap_after(idx) -> usize`：计算某区域之后到下一区域或 `KERN_BASE` 的空洞大小。

### `ZoneInfo`

职责：模拟 DMA/NORMAL/HIGH 等内存 zone 的容量、水位线和压力。

字段：

- `zone_id: usize`：zone 编号。
- `base_pfn: usize`：zone 起始页帧号。
- `page_count: usize`：zone 包含的页数。
- `free_count: AtomicUsize`：zone 内空闲页数。
- `low_watermark: usize`：低水位线，低于或等于时表示压力高。
- `high_watermark: usize`：高水位线，达到时压力为 0。
- `managed: AtomicBool`：是否由分配器管理。

方法：

- `new(id, base, count, low, high) -> Self`：创建 zone，并把空闲页设为 `count`。
- `zone_can_alloc() -> bool`：空闲数高于低水位线时允许分配。
- `zone_pressure() -> usize`：返回 0 到 100 的压力百分比。
- `reclaim_target() -> usize`：返回为了恢复到高水位线需要回收的页数。
- `contains_pfn(pfn) -> bool`：判断页帧号是否落在 zone 内。

### `BuddyStatistics`

职责：buddy 分配器通用统计。

字段：

- `alloc_count: usize`：成功分配次数。
- `free_count: usize`：成功释放次数。
- `split_count: usize`：拆分大块次数。
- `merge_count: usize`：合并 buddy 次数。
- `exact_hit_count: usize`：请求 order 与来源 order 相同的命中次数。
- `failed_alloc_count: usize`：失败分配次数。

方法：

- 无独立方法；`BuddyAllocator` 和 `HeuristicAllocator` 直接更新并返回该统计。

### `BuddyAllocator`

职责：经典 buddy 页帧分配器，按 order 管理空闲块，支持对齐分配、释放合并和碎片统计。

字段：

- `free_lists: Vec<Vec<usize>>`：每个 order 的空闲块起始地址列表。
- `max_order: usize`：最大 order。
- `base_addr: usize`：管理区起始地址。
- `total_pages: usize`：管理总页数。
- `allocated: AtomicUsize`：已分配页数。
- `addr_order_map: BTreeMap<usize, usize>`：已分配块地址到真实 order 的映射，释放时用于校验。
- `statistics: BuddyStatistics`：分配器统计。

方法：

- `new(base, total_pages, max_order) -> Self`：初始化 free lists，把可管理内存切成若干 buddy 块。
- `alloc_order(order) -> Option<usize>`：按 order 分配，默认页对齐。
- `alloc_order_aligned(order, align) -> Option<usize>`：分配满足页数对齐约束的块；必要时拆分更高 order 块。
- `alloc_page() -> Option<usize>`：分配单页，即 order 0。
- `alloc_pages(pages) -> Option<usize>`：分配能覆盖 `pages` 的连续块。
- `free(addr)`：释放已分配块，并尽可能向上合并 buddy。
- `free_pages_count() -> usize`：统计所有 free lists 中的空闲页数。
- `is_free_addr(addr) -> bool`：判断地址是否落在某个空闲块中。
- `largest_free_order() -> usize`：返回当前最大可用 order。
- `fragmentation_score() -> usize`：以最大空闲块占总空闲页比例估算碎片率。
- `snapshot() -> BuddyAllocator`：复制分配器状态，便于测试观察。

### `HeuristicPolicy`

职责：启发式分配器的策略参数集合。

字段：

- `order0_base_target`、`order1_base_target`、`order2_base_target`、`order3_base_target`：低阶 order 的基础保留目标。
- `order0_max_target`、`order1_max_target`、`order2_max_target`、`order3_max_target`：动态反馈能提高到的目标上限。
- `high_order_target: usize`：高阶块保留目标。
- `protect_min_order: usize`：从哪个 order 开始视为高阶块并保护。
- `feedback_window_ops: usize`：多少次分配请求后更新一次动态目标。
- `feedback_pool_fraction: usize`：保留目标占总池容量的分母，用于小池缩放。
- `pressure_decay_divisor: usize`：进入高阶压力时动态目标衰减因子。

方法/Trait：

- `Default::default()`：提供一组保守默认策略。

### `HeuristicStatistics`

职责：启发式行为统计。

字段：

- `free_list_alloc_count: usize`：从 free list 成功分配次数。
- `preserve_count: usize`：因保留目标而停止合并次数。
- `active_coalesce_count: usize`：主动合并成功次数。
- `active_coalesce_pass_count: usize`：主动合并扫描轮数。
- `pressure_enter_count: usize`：进入高阶块压力模式次数。
- `pressure_exit_count: usize`：退出压力模式次数。
- `feedback_update_count: usize`：动态目标更新次数。
- `pressure_decay_count: usize`：压力下目标衰减次数。

方法：

- 无独立方法；由 `HeuristicAllocator` 维护。

### `HeuristicAllocator`

职责：在 buddy 分配器基础上加入低阶块保留、高阶块保护、反馈目标和主动合并。

字段：

- `free_lists: Vec<Vec<usize>>`：每个 order 的空闲块列表。
- `max_order: usize`：最大 order。
- `base_addr: usize`：管理区起始地址。
- `total_pages: usize`：管理总页数。
- `allocated: AtomicUsize`：已分配页数。
- `addr_order_map: BTreeMap<usize, usize>`：已分配块地址到真实 order 的映射。
- `statistics: BuddyStatistics`：buddy 基础统计。
- `policy: HeuristicPolicy`：启发式策略。
- `heuristic_statistics: HeuristicStatistics`：启发式统计。
- `dynamic_targets: [usize; 4]`：order 0 到 3 的动态保留目标。
- `recent_allocs: [usize; 4]`：反馈窗口内的低阶请求计数。
- `feedback_ops: usize`：当前反馈窗口累计操作数。
- `merge_pressure: bool`：是否处于高阶块压力模式。

公开方法：

- `new(base, total_pages, max_order) -> Self`：使用默认策略创建分配器。
- `with_policy(base, total_pages, max_order, policy) -> Self`：使用指定策略创建分配器。
- `alloc_order(order) -> Option<usize>`：按 order 分配。
- `alloc_order_aligned(order, align) -> Option<usize>`：按 order 和页对齐分配；会综合拆分成本、保留目标和局部拓扑评分。
- `alloc_page() -> Option<usize>`：分配单页。
- `alloc_pages(pages) -> Option<usize>`：分配覆盖指定页数的连续块。
- `free(addr)`：释放块；普通释放后可能退出压力模式。
- `free_pages_count() -> usize`：统计空闲页数。
- `is_free_addr(addr) -> bool`：判断地址是否位于空闲块。
- `largest_free_order() -> usize`：返回最大可用 order。
- `fragmentation_score() -> usize`：计算碎片率。
- `allocator_statistics() -> BuddyStatistics`：返回 buddy 统计快照。
- `heuristic_statistics() -> HeuristicStatistics`：返回启发式统计快照。
- `snapshot() -> HeuristicAllocator`：复制当前状态。

私有方法：

- `free_to_buddy(addr, real_order, count_as_free) -> bool`：把块放回 buddy 系统并按策略决定是否合并。
- `try_fast_exact_alloc(order, align_pages) -> Option<usize>`：优先从同 order 中选不破坏可合并性的块。
- `alloc_from_free_list(request_order, source_order, pos, block, target_addr) -> usize`：从指定 free list 移除块并拆分到请求 order。
- `target_blocks_at_order(order) -> usize`：读取某 order 的保留目标。
- `policy_base_targets(policy) -> [usize; 4]`：提取策略基础目标。
- `policy_max_targets(policy) -> [usize; 4]`：提取策略最大目标并保证不低于基础目标。
- `scaled_targets_for_pool(total_pages, feedback_pool_fraction, raw_targets) -> [usize; 4]`：按池大小缩放目标。
- `clamp_targets_to_policy_and_pool(raw_targets) -> [usize; 4]`：把动态目标限制在策略范围和池容量预算内。
- `scaled_base_targets() -> [usize; 4]`：返回按池大小缩放后的基础目标。
- `record_alloc_request(order)`：记录近期分配请求并尝试触发反馈更新。
- `maybe_update_feedback_targets()`：反馈窗口达到阈值时重算目标。
- `recompute_dynamic_targets()`：根据近期请求分布调整低阶保留目标。
- `decay_dynamic_targets_for_pressure()`：高阶压力下衰减低阶保留目标。
- `should_preserve_order_block(order) -> bool`：判断释放合并时是否需要保留某 order 的块。
- `buddy_addr(addr, order) -> Option<usize>`：计算 buddy 地址并做范围检查。
- `find_free_buddy_pos(addr, order) -> Option<(usize, usize)>`：查找某块的空闲 buddy 及其位置。
- `has_free_buddy_at_order(addr, order) -> bool`：判断同 order buddy 是否空闲。
- `effective_free_blocks_at_order(order) -> usize`：把更高 order 可拆块也计入后的有效可用块数。
- `exact_free_blocks_at_order(order) -> usize`：只统计当前 order 的空闲块数。
- `exact_deficit_at_order(order) -> usize`：当前 order 相对保留目标的缺口。
- `exact_surplus_at_order(order) -> usize`：当前 order 相对保留目标的盈余。
- `first_high_order_under_target() -> Option<usize>`：找出第一个低于目标的高阶 order。
- `high_order_under_target() -> bool`：判断是否存在高阶块不足。
- `enter_merge_pressure()`：进入压力模式并衰减低阶动态目标。
- `leave_merge_pressure()`：高阶目标恢复后退出压力模式。
- `try_coalesce_order_once(order) -> bool`：在某 order 主动合并一对 buddy。
- `coalesce_free_lists_for_pressure() -> bool`：压力模式下反复扫描并主动合并。
- `alloc_candidate_score(request_order, source_order, candidate_addr, target_addr) -> isize`：为候选块打分，综合拆分、保留和局部拓扑。
- `projected_exact_deficit_penalty(order, projected_exact) -> isize`：评估取走候选块后造成的保留缺口惩罚。
- `min_possible_alloc_score(request_order, source_order) -> isize`：估计某来源 order 理论最优分数，用于提前停止扫描。
- `local_alloc_topology_penalty(request_order, source_order, candidate_addr, target_addr) -> isize`：惩罚破坏可合并 buddy 的精确分配。

### `FramePool`

职责：页帧池门面，把 `HeuristicAllocator` 的地址分配转换为 frame id 级别接口。

字段：

- `allocator: Mutex<HeuristicAllocator>`：底层启发式分配器。
- `cap: usize`：总 frame 数。

方法：

- `new(n) -> Self`：创建管理 `n` 个 frame 的池。
- `get(id) -> Option<usize>`：进入 `GKL` 后调用 `get_inner`，返回 frame id。
- `get_inner() -> Option<usize>`：分配单页并把地址转换为 frame id。
- `get_contig(sz, align_log2) -> Option<usize>`：分配连续 `sz` 页，并按 `2^align_log2` 页对齐。
- `put(idx)`：释放 frame id。
- `avail(idx) -> bool`：判断 frame id 是否空闲。
- `free_count() -> usize`：返回空闲 frame 数。
- `get_zone_aware(zone) -> Option<usize>`：在指定 zone 内分配单页，并同步 zone 空闲计数。
- `put_zone_aware(idx, zone)`：释放指定 zone 内的 frame。
- `batch_alloc(count) -> Vec<usize>`：批量分配不要求连续的 frame。

相关自由函数：

- `frame_alloc(pool) -> Option<usize>`：分配单页并返回 `MEM_OFF + frame * PAGE_SZ` 地址。
- `frame_dealloc(pool, target)`：按地址释放页帧，忽略未对齐或越界地址。
- `frame_alloc_contig(pool, sz, align) -> Option<usize>`：分配连续页并返回起始地址。

### `SharedPage`

职责：COW 共享页模拟对象，跟踪当前 frame、是否可写、是否还处于待处理 COW 状态。

字段：

- `frame: AtomicUsize`：当前 frame id。
- `w: AtomicBool`：是否已经变为可写。
- `pending: AtomicBool`：是否仍等待 COW fault 处理。

方法：

- `new(f) -> Self`：以初始 frame id 创建 pending COW 页。
- `fault(pool, src) -> Result<usize, &'static str>`：若已处理则返回当前 frame；否则分配新 frame，减少源页引用计数，并标记为可写。
- `is_cow_resolved() -> bool`：判断 COW 是否已经完成。
- `frame_id() -> usize`：读取当前 frame id。

### `KStk`

职责：用 boxed slice 模拟内核栈。

字段：

- tuple 字段 `usize`：内核栈底部原始指针地址。

方法：

- `new() -> Self`：分配 `KSTK_SZ` 字节并保存裸指针。
- `top() -> usize`：返回栈顶地址。

Trait：

- `Drop::drop()`：根据保存的裸指针重建 boxed slice 并释放内存。

### `SlabEntry`

职责：简单 slab 对象页，维护固定大小对象的空闲链表和调试信息。

字段：

- `data: Vec<u8>`：slab 原始字节存储。
- `obj_size: usize`：按 `SLAB_ALIGN` 对齐后的对象大小。
- `capacity: usize`：对象总容量。
- `free_list: VecDeque<usize>`：可用对象偏移列表。
- `allocated: usize`：当前已分配对象数。
- `tag: u32`：调试或分类标签。

方法：

- `new(obj_size, capacity) -> Self`：创建 slab，并把所有对象偏移加入 free list。
- `slab_alloc(zeroed) -> Option<usize>`：弹出一个空闲对象偏移；当前实现会在 `zeroed == false` 时清零该对象区域。
- `slab_free(offset)`：验证偏移合法且对齐后放回 free list，并减少已分配计数。
- `slab_used() -> usize`：返回已分配对象数。
- `slab_avail() -> usize`：返回空闲对象数。
- `shrink() -> usize`：若无对象占用，则清空底层数据并返回释放字节数。
- `obj_at(offset) -> Option<&[u8]>`：返回对象只读切片。
- `obj_at_mut(offset) -> Option<&mut [u8]>`：返回对象可写切片。

### `AddrSpace`

职责：更完整的进程地址空间对象，组合 `VmMap`、页表根、ASID、引用计数和 COW 页表。

字段：

- `vm_map: VmMap`：虚拟内存区域表。
- `page_table_root: usize`：模拟页表根地址。
- `asid: u16`：地址空间 id。
- `ref_count: AtomicUsize`：地址空间引用计数。
- `cow_pages: Mutex<BTreeMap<usize, PgFrame>>`：页地址到 COW 页帧引用计数的映射。

方法：

- `new(asid) -> Self`：创建空地址空间。
- `fork_from(parent, new_asid) -> Self`：复制父地址空间区域和 COW 页记录，写区域会提升引用计数。
- `handle_cow_fault(addr, pool) -> Result<usize, &'static str>`：处理写时复制 fault；必要时分配新 frame。
- `unmap_range(start, len) -> usize`：删除 VMA 和 COW 页记录，返回释放/删除数量。
- `protect(start, len, new_flags) -> Result<(), &'static str>`：修改与范围相交区域的权限标志。
- `rss_pages() -> usize`：返回 COW 页记录数量，近似 RSS。
- `cow_sharers() -> usize`：返回引用计数大于 1 的 COW 页数量。
- `split_region(addr) -> Result<(), &'static str>`：在区域内部地址拆出一个新 `VmRegion`。

相关自由函数：

- `p2v(pa) -> usize`：物理地址到 `PHYS_OFF` 线性映射地址。
- `v2p(va) -> usize`：线性映射虚拟地址转物理地址。
- `k_off(va) -> usize`：计算相对 `KERN_BASE` 的偏移。
- `check_access(addr, len) -> bool`：检查用户地址范围不越过 `KERN_BASE` 且无溢出。
- `check_access_rw(addr, len, writable) -> bool`：读写访问检查，额外处理零长度、页跨度和写对齐提示。
- `cfu<T>(addr, len) -> Option<T>`：模拟 copy-from-user，合法时返回默认值。
- `ctu<T>(addr, len, v) -> bool`：模拟 copy-to-user，只做访问检查。
- `rdu_fixup() -> usize`：占位式读用户修复逻辑，当前固定返回 1。
- `heap_init(base, sz) -> usize`：对齐堆基址和大小，返回堆尾。
- `heap_grow(pool, n) -> Vec<(usize, usize)>`：从页帧池分配 `n` 页，并把相邻虚拟段合并。

## K3：字节缓冲、终端与通道

### `CircBuf`

职责：固定容量环形缓冲区，供 `Channel` 和基础 ring buffer 测试使用。

字段：

- `data: Vec<u8>`：底层存储。
- `rd: usize`：读游标。
- `wr: usize`：写游标。
- `cap: usize`：容量。
- `n: usize`：当前元素数量。

方法：

- `new(c) -> Self`：创建容量为 `c` 的环形缓冲。
- `with_pos(c, r, w) -> Self`：用指定读写游标创建缓冲，并推算当前长度。
- `push(v) -> bool`：写入一个字节；满或索引异常时返回 `false`。
- `pop() -> Option<u8>`：读出一个字节；空时返回 `None`。
- `len() -> usize`：返回当前元素数。
- `empty() -> bool`：判断是否为空。
- `full() -> bool`：判断是否已满。
- `peek() -> Option<u8>`：查看下一个可读字节但不移动游标。
- `drain_to(dst, max) -> usize`：最多弹出 `max` 字节追加到 `dst`。
- `fill_from(src) -> usize`：从切片写入直到满，返回写入数。
- `remaining() -> usize`：返回剩余容量。

### `TrmIO`

职责：模拟 Linux termios 结构，供 `ioctl(TCGETS/TCSETS)` 参数大小检查使用。

字段：

- `iflag: u32`：输入模式标志。
- `oflag: u32`：输出模式标志。
- `cflag: u32`：控制模式标志。
- `lflag: u32`：本地模式标志。
- `line: u8`：line discipline。
- `cc: [u8; 32]`：控制字符数组。
- `ispeed: u32`：输入速度。
- `ospeed: u32`：输出速度。

方法/Trait：

- `Default::default()`：返回一组常见终端默认标志和控制字符。

### `WinSz`

职责：终端窗口大小结构，供 `ioctl(TIOCGWINSZ)` 参数大小检查使用。

字段：

- `row: u16`：行数。
- `col: u16`：列数。
- `xpx: u16`：水平像素。
- `ypx: u16`：垂直像素。

方法/Trait：

- 派生 `Default`，默认全 0。

### `Channel`

职责：带环形缓冲、轻量锁和等待队列的生产者/消费者通道。

字段：

- `buf: Mutex<CircBuf>`：通道数据缓冲。
- `guard: Spin`：保护短临界区的自旋锁。
- `wq: SyncQueue`：接收方等待队列。
- `shut: AtomicBool`：通道关闭标志。

方法：

- `new(cap) -> Self`：创建通道；容量 0 会提升到 1，过大容量会限制到 `1 << 20`。
- `recv() -> Option<u8>`：阻塞式接收；无数据且未关闭时 park，关闭且无数据时返回 `None`。
- `send(v) -> bool`：发送一个字节；成功后唤醒一个等待者。
- `close()`：设置关闭标志并唤醒所有等待者。
- `try_recv() -> Option<u8>`：非阻塞尝试接收，锁忙或无数据返回 `None`。
- `send_batch(data) -> usize`：批量写入，成功写入后唤醒一个等待者。
- `depth() -> usize`：返回当前缓冲深度。
- `drain_all() -> Vec<u8>`：取出所有可读数据。
- `is_closed() -> bool`：读取关闭状态。
- `remaining_capacity() -> usize`：返回剩余容量。

相关自由函数/方法：

- `ser(c) -> u8`：把回车 `\r` 规范化为换行 `\n`。
- `Kernel::tty_push(c)`：把字节规范化后推入内核 TTY 缓冲。
- `Kernel::tty_pop() -> Option<u8>`：从内核 TTY 缓冲弹出一个字节。

## K4：文件描述符、文件、管道与 epoll

### `FdOpt`

职责：文件描述符访问选项。

字段：

- `rd: bool`：允许读。
- `wr: bool`：允许写。
- `ap: bool`：追加写。
- `nb: bool`：非阻塞。

方法/Trait：

- `Default::default()`：默认只读、非追加、阻塞。

### `FdState`

职责：文件描述符共享状态，多个 dup 出来的 `FHandle` 共享 offset 和选项。

字段：

- `off: u64`：当前文件偏移。
- `opt: FdOpt`：描述符选项。
- `flk: u8`：文件锁状态占位。

方法：

- `create(opt) -> Arc<RwLock<Self>>`：创建带读写锁的共享描述符状态。

### `FHandle`

职责：普通文件模拟对象，组合路径、内容、描述符状态、pipe 标记和 close-on-exec 标记。

字段：

- `path: String`：文件路径。
- `data: Arc<Mutex<Vec<u8>>>`：文件内容。
- `desc: Arc<RwLock<FdState>>`：共享 fd 状态。
- `pipe: bool`：是否以 pipe 语义创建的标记。
- `cloexec: bool`：exec 时是否关闭。

方法：

- `new(path, opt, pipe, cloexec) -> Self`：创建空内容文件。
- `with_data(path, opt, d) -> Self`：以已有数据创建文件。
- `dup(cloexec) -> Self`：共享内容和 fd 状态，生成新 handle。
- `set_opt(arg)`：根据 flag 设置非阻塞位。
- `get_opt() -> FdOpt`：读取描述符选项。
- `read(buf) -> Result<usize, &'static str>`：从当前 offset 读取并推进 offset。
- `read_at(off, buf) -> Result<usize, &'static str>`：从指定偏移读取，不推进共享 offset。
- `write(buf) -> Result<usize, &'static str>`：从当前 offset 或文件尾写入并推进 offset。
- `write_at(off, buf) -> Result<usize, &'static str>`：从指定偏移写入，必要时扩展文件。
- `seek(pos) -> Result<u64, &'static str>`：按 `FSeek` 更新 offset。
- `transfer(dir, offset, buf_rd, buf_wr) -> Result<usize, &'static str>`：统一读写入口；`dir & 1 != 0` 表示读，否则写。
- `set_len(len) -> Result<(), &'static str>`：调整文件长度，要求可写。
- `sync_all() -> Result<(), &'static str>`：模拟全量同步，当前直接成功。
- `sync_data() -> Result<(), &'static str>`：模拟数据同步，当前直接成功。
- `metadata_sz() -> usize`：返回文件长度。
- `lookup(path, depth) -> Result<(), &'static str>`：路径查找占位，当前直接成功。
- `read_entry() -> Result<String, &'static str>`：模拟目录项读取，返回 `entry_N` 并推进 offset。
- `poll_status() -> (bool, bool, bool)`：返回普通文件可读、可写、无错误。
- `io_ctl(cmd, arg) -> Result<usize, &'static str>`：普通文件 ioctl 占位，当前返回 0。
- `mmap(start, end, off) -> Result<(), &'static str>`：文件映射占位，当前直接成功。
- `inode_ref() -> Arc<Mutex<Vec<u8>>>`：返回内容引用。
- `advise_readahead(offset, len) -> Result<(), &'static str>`：模拟 readahead 计算，当前直接成功。
- `fallocate(offset, len) -> Result<(), &'static str>`：预分配文件范围，要求可写。
- `splice_to(dst, count) -> Result<usize, &'static str>`：从当前 offset 复制最多 `count` 字节写入目标文件。

Trait：

- `fmt::Debug`：输出路径和 offset。

### `FSeek`

职责：文件 seek 的位置模式。

变体：

- `Start(u64)`：从文件起始设置 offset。
- `End(i64)`：从文件尾加偏移。
- `Cur(i64)`：从当前 offset 加偏移。

### `PipeBuf`

职责：管道读写端共享的缓冲和状态。

字段：

- `buf: VecDeque<u8>`：管道字节队列。
- `bus: EvBus`：管道可读/关闭事件。
- `ends: i32`：剩余端点数量，初始为 2。

方法：

- 无独立方法；由 `PipeNode` 操作。

### `PipeDir`

职责：区分管道端方向。

变体：

- `Rd`：读端。
- `Wr`：写端。

### `PipeNode`

职责：管道端点，读端和写端共享同一个 `PipeBuf`。

字段：

- `data: Arc<Mutex<PipeBuf>>`：共享管道缓冲。
- `dir: PipeDir`：当前端点方向。

方法：

- `pair() -> (PipeNode, PipeNode)`：创建一对读端/写端。
- `can_read() -> bool`：读端有数据或对端关闭时可读。
- `can_write() -> bool`：写端且两端仍存在时可写。
- `read_at(buf) -> Result<usize, &'static str>`：从读端读取；空且两端都在时返回 `"again"`。
- `write_at(buf) -> Result<usize, &'static str>`：向写端写入并设置 `READABLE`。
- `poll() -> (bool, bool, bool)`：返回可读、可写、错误状态。

Trait：

- `Drop::drop()`：端点析构时减少 `ends` 并发布 `CLOSED`。

### `FLike`

职责：fd table 中的统一文件对象枚举，屏蔽普通文件、管道、epoll 实例差异。

变体：

- `File(FHandle)`：普通文件。
- `Pipe(PipeNode)`：管道端点。
- `Ep(EpInst)`：epoll 实例。

方法：

- `dup(cloexec) -> FLike`：按底层类型复制 handle；文件共享 offset，管道共享缓冲，epoll 共享 ready/new_ctl 集合。
- `read(buf) -> Result<usize, &'static str>`：按底层类型读取；epoll 读返回 `"enosys"`。
- `write(buf) -> Result<usize, &'static str>`：按底层类型写入；epoll 写返回 `"enosys"`。
- `io_ctl(req, a1) -> Result<usize, &'static str>`：普通文件转发 ioctl，管道支持少量请求，epoll 返回 `"enosys"`。
- `mmap_fl(start, end, off) -> Result<(), &'static str>`：普通文件 mmap；其他类型返回 `"enosys"`。
- `poll() -> (bool, bool, bool)`：统一返回可读、可写、错误状态。

Trait：

- `fmt::Debug`：输出 `F(...)`、`P` 或 `E`。

### `PseudoNode`

职责：只读伪文件节点。

字段：

- `content: Vec<u8>`：伪文件内容。
- `ftype: u8`：伪文件类型标记。

方法：

- `new(s, ft) -> Self`：用字符串内容和类型创建节点。
- `read_at(off, buf) -> usize`：从指定偏移读取。
- `write_at(off, buf) -> Result<usize, &'static str>`：写入不支持，返回 `"nosup"`。
- `metadata_sz() -> usize`：返回内容长度。

### `EpData`

职责：epoll 用户数据。

字段：

- `ptr: u64`：用户传入的 opaque 数据。

方法：

- 无独立方法。

### `EpEvent`

职责：epoll 事件描述。

字段：

- `events: u32`：事件位图。
- `data: EpData`：用户数据。

关联常量：

- `IN`、`OUT`、`ERR`、`HUP`、`PRI`、`RDNORM`、`RDBAND`、`WRNORM`、`WRBAND`、`MSG`、`RDHUP`、`EXCL`、`WAKEUP`、`ONESHOT`、`ET`：常见 epoll 事件位。

方法：

- `has(ev) -> bool`：判断事件位是否包含 `ev`。

### `EpCtlOp`

职责：epoll control 操作号命名空间。

字段：

- 无字段。

关联常量：

- `ADD = 1`：添加 fd。
- `DEL = 2`：删除 fd。
- `MOD = 3`：修改 fd。

### `EpInst`

职责：epoll 实例，维护 fd 到事件的映射，以及 ready/new_ctl 集合。

字段：

- `events: BTreeMap<usize, EpEvent>`：fd 到监听事件的映射。
- `ready: Arc<Mutex<BTreeSet<usize>>>`：就绪 fd 集合。
- `new_ctl: Arc<Mutex<BTreeSet<usize>>>`：最近 add/mod 的 fd 集合。

方法：

- `new() -> Self`：创建空 epoll 实例。
- `control(op, fd, ev) -> Result<(), &'static str>`：支持 ADD、MOD、DEL；MOD/DEL 要求 fd 已存在。

相关自由函数：

- `read_as_vec(data) -> Vec<u8>`：把切片复制成 Vec。
- `audit_fd_table(files) -> Vec<usize>`：扫描 fd 表空洞、异常 pipe 和空路径文件，返回可疑 fd/空洞。

## K5：缓存、对象注册、挂载、I/O 调度与磁盘

### `PageCacheEntry`

职责：页缓存条目。

字段：

- `page_id: usize`：页编号。
- `data: Vec<u8>`：页内容。
- `dirty: bool`：是否脏页。
- `access_tick: usize`：最近访问 tick。
- `pin_count: usize`：pin 计数，非零时不应被 LRU 淘汰。

方法：

- 无独立方法；由 `PageCache` 操作。

### `PageCache`

职责：简单 LRU 页缓存，带命中、未命中和淘汰统计。

字段：

- `entries: HashMap<usize, PageCacheEntry>`：page id 到缓存条目。
- `capacity: usize`：最大条目数。
- `hits: AtomicUsize`：命中次数。
- `misses: AtomicUsize`：未命中次数。
- `evictions: AtomicUsize`：淘汰次数。
- `lru_order: VecDeque<usize>`：LRU 顺序，队尾为最近访问。

方法：

- `new(capacity) -> Self`：创建指定容量缓存。
- `lookup(page_id) -> Option<&[u8]>`：查找页；命中更新 LRU 和访问 tick，未命中增加 misses。
- `insert(page_id, data)`：插入页；容量满时先尝试 LRU 淘汰。
- `evict_lru() -> bool`：淘汰第一个未 pinned 的 LRU 页。
- `mark_dirty(page_id)`：标记脏页。
- `writeback_all() -> usize`：清除所有 dirty 标记，返回写回数量。
- `stats() -> (usize, usize, usize)`：返回 hits、misses、evictions。
- `pin(page_id) -> bool`：增加 pin 计数。
- `unpin(page_id) -> bool`：减少 pin 计数。
- `invalidate(page_id) -> bool`：删除指定页。
- `flush_range(start, end) -> usize`：清除指定 page id 范围内 dirty 标记。

### `KObjEntry`

职责：内核对象注册表中的对象记录。

字段：

- `obj_id: usize`：对象 id。
- `type_tag: u32`：对象类型标签。
- `owner_pid: usize`：所属进程 id。
- `created_tick: usize`：创建时间 tick。
- `ref_count: usize`：对象引用计数。
- `parent_id: Option<usize>`：父对象 id，用于对象图。

方法：

- 无独立方法；由 `KObjRegistry` 管理。

### `KObjRegistry`

职责：内核对象注册表，维护对象 id、类型索引和父子关系。

字段：

- `objects: Mutex<BTreeMap<usize, KObjEntry>>`：对象 id 到对象记录。
- `seq: AtomicUsize`：下一个对象 id。
- `type_index: Mutex<BTreeMap<u32, Vec<usize>>>`：类型标签到对象 id 列表。

方法：

- `new() -> Self`：创建空注册表。
- `register(type_tag, owner_pid) -> usize`：注册无父对象并返回 id。
- `register_child(type_tag, owner_pid, parent) -> usize`：注册有父对象并返回 id。
- `unregister(id) -> bool`：删除对象并同步类型索引。
- `find_by_type(tag) -> Vec<usize>`：按类型查找对象 id。
- `dump_graph() -> Vec<(usize, usize)>`：返回父子边列表。
- `gc_sweep() -> usize`：删除引用计数为 0 的对象。
- `ref_up(id) -> bool`：引用计数加一。
- `ref_down(id) -> bool`：引用计数饱和减一。
- `count() -> usize`：对象总数。
- `owner_objects(pid) -> Vec<usize>`：列出某 owner 的对象。

### `CacheSlot`

职责：块缓存中的单个块条目。

字段：

- `id: usize`：块 id。
- `payload: Vec<u8>`：块数据，模拟为 512 字节。
- `modified: bool`：是否脏块。

方法：

- 无独立方法。

### `CacheChain`

职责：块缓存哈希桶，每条链有自旋锁和条目列表。

字段：

- `lk: Spin`：链级自旋锁。
- `items: Mutex<Vec<CacheSlot>>`：链内缓存条目。

方法：

- `new() -> Self`：创建空链。

### `BlockCache`

职责：哈希链式块缓存，支持 fetch、同步、失效和统计。

字段：

- `chains: Vec<CacheChain>`：哈希链数组。
- `width: usize`：链数量。

方法：

- `new(w) -> Self`：创建 `w` 条链。
- `idx(k) -> usize`：计算块 id 到链索引。
- `fetch(k, lat) -> Option<Vec<u8>>`：查找块；命中复制 payload，未命中可模拟延迟并生成 512 字节数据后插入。
- `sync_all(id)`：进入 `GKL` 后清除所有 modified 标记。
- `invalidate(k)`：删除指定块 id。
- `total_entries() -> usize`：统计所有链条目数量。
- `dirty_count() -> usize`：统计脏块数量。
- `evict_cold(max_age) -> usize`：按模拟 age 删除冷块，返回淘汰数量。

### `MountEntry`

职责：挂载表中的一条 prefix 到 target 映射。

字段：

- `prefix: String`：被挂载路径前缀。
- `target: String`：目标设备或目标路径。

方法：

- 无独立方法。

### `MountTable`

职责：挂载表，支持最长前缀匹配解析。

字段：

- `entries: RwLock<Vec<MountEntry>>`：挂载项列表，按 prefix 长度降序维护。

方法：

- `new() -> Self`：创建空挂载表。
- `bind(pfx, tgt)`：添加挂载项，避免完全重复，并重新按 prefix 长度排序。
- `resolve(path) -> Result<String, &'static str>`：选择最长前缀匹配并递归解析剩余路径；无匹配时规整多余斜杠。
- `unmount(pfx) -> bool`：删除指定 prefix 的挂载项。
- `list_mounts() -> Vec<(String, String)>`：列出所有挂载。
- `find_mount(path) -> Option<MountEntry>`：返回路径的最长匹配挂载项。
- `mount_count() -> usize`：返回挂载数。
- `has_prefix(pfx) -> bool`：判断 prefix 是否存在。

### `IoRequest`

职责：块 I/O 请求。

字段：

- `block: usize`：目标块号。
- `write: bool`：是否写请求。
- `priority: u8`：请求优先级。
- `submitted_tick: usize`：提交时间 tick。

方法：

- 无独立方法。

### `IoQueue`

职责：块 I/O 调度队列，模拟 elevator/SCAN 调度和相邻请求合并。

字段：

- `pending: Mutex<VecDeque<IoRequest>>`：待调度请求。
- `head_pos: AtomicUsize`：模拟磁头当前位置。
- `direction_up: AtomicBool`：当前扫描方向。
- `dispatched: AtomicUsize`：已派发请求数。
- `merged: AtomicUsize`：合并请求数。

方法：

- `new() -> Self`：创建空队列。
- `submit(blk, write, priority)`：提交单个请求。
- `submit_batch(requests) -> usize`：批量提交请求；队列过深时触发合并。
- `dispatch() -> Option<(usize, bool)>`：按当前方向选择距离最近的请求，更新磁头和方向。
- `merge_adjacent() -> usize`：合并相邻且读写方向相同的请求。
- `depth() -> usize`：返回待处理请求数量。

### `Disk`

职责：块设备模拟，支持可配置错误、重试和 journal 设备。

字段：

- `errs: AtomicUsize`：剩余错误次数；`usize::MAX` 表示持续失败。
- `ops: AtomicUsize`：操作计数。
- `label: String`：磁盘标签。
- `journal: Option<Arc<Disk>>`：可选 journal 设备。

方法：

- `new(s) -> Self`：创建正常磁盘。
- `failing(s, n) -> Self`：创建初始会失败 `n` 次的磁盘。
- `attach_journal(d)`：挂接 journal 设备。
- `set_errs(n)`：设置剩余错误次数。
- `read_block(blk, out) -> Result<(), &'static str>`：循环读块，错误时消耗错误计数或尝试 journal。
- `read_block_n(blk, out, lim) -> Result<usize, &'static str>`：带最大尝试次数的读块，成功返回尝试次数。
- `total_ops() -> usize`：读取操作计数。
- `reset_ops()`：清零操作计数。
- `write_block(blk, data) -> Result<(), &'static str>`：模拟写块，错误计数非零时返回 `"io_error"`。
- `flush() -> Result<(), &'static str>`：模拟 flush，同时给 journal 增加操作计数。

## K6：System V IPC 与共享内存

### `IpcPerm`

职责：System V IPC 权限元数据。

字段：

- `key: u32`：IPC key。
- `uid: u32`、`gid: u32`：当前 owner 用户/组。
- `cuid: u32`、`cgid: u32`：创建者用户/组。
- `mode: u32`：权限位。
- `seq: u32`：序列号。
- `pad1: usize`、`pad2: usize`：布局填充。

方法：

- 无独立方法。

### `SemDs`

职责：System V semaphore array 描述结构。

字段：

- `perm: IpcPerm`：权限信息。
- `otime: usize`：最后 semop 时间。
- `_p1: usize`：填充。
- `ctime: usize`：最后变更时间。
- `_p2: usize`：填充。
- `nsems: usize`：信号量数量。

方法：

- 无独立方法；由 `SemArr` 管理。

### `SemArr`

职责：System V semaphore array，一组 `Sema` 加描述信息。

字段：

- `ds: Mutex<SemDs>`：数组元数据。
- `sems: Vec<Sema>`：实际信号量数组。

方法：

- `remove()`：删除数组中的所有信号量。
- `otime_now()`：更新 `otime`；当前实现写 0。
- `ctime_now()`：更新 `ctime`；当前实现写 0。
- `set_ds(new)`：只允许更新 uid、gid、mode 等权限元数据。
- `get_or_create(key, nsems, flags, store) -> Result<Arc<Self>, &'static str>`：按 key 从 weak store 复用数组，或创建新数组；处理私有 key 和 `IPC_EXCL` 类标志。

Trait：

- `Index<usize, Output = Sema>`：允许 `arr[i]` 访问第 i 个信号量。

### `SemCtx`

职责：进程私有 semaphore 上下文，保存 semid 到 `SemArr` 的映射和 undo 记录。

字段：

- `arrays: BTreeMap<SemId, Arc<SemArr>>`：进程内 semid 到 semaphore array。
- `undos: BTreeMap<(SemId, SemNum), SemOp>`：退出时需要回滚的 semaphore 操作。

方法：

- `add(arr) -> SemId`：分配一个本进程空闲 semid 并记录数组。
- `remove(id)`：移除 semid。
- `free_id() -> SemId`：私有方法，寻找空闲 semid。
- `get(id) -> Option<Arc<SemArr>>`：按 semid 获取数组。
- `add_undo(id, num, op)`：累加 undo 操作，内部存储为相反操作。

Trait：

- `Clone::clone()`：继承数组映射，但清空 undo 记录。
- `Drop::drop()`：按 undo 记录释放对应信号量，目前只处理 op 为 1 的情况。

### `ShmTag`

职责：进程内共享内存段标签。

字段：

- `addr: usize`：该段附着到进程地址空间的虚拟地址。
- `pages: Arc<Mutex<Vec<usize>>>`：共享页集合。

方法：

- `set_addr(a)`：更新附着地址。

### `ShmCtx`

职责：进程私有共享内存上下文。

字段：

- `ids: BTreeMap<ShmId, ShmTag>`：进程内 shmid 到共享内存标签。

方法：

- `add(g) -> ShmId`：分配一个本进程空闲 shmid 并记录共享页集合。
- `get(id) -> Option<ShmTag>`：按 shmid 获取标签。
- `set(id, tag)`：设置或覆盖标签。
- `get_id_by_addr(addr) -> Option<ShmId>`：按附着地址查找 shmid。
- `pop(id)`：移除 shmid。

Trait：

- `Clone::clone()`：复制进程内 id 到 tag 的映射。

相关自由函数/类型别名：

- `type SemId = usize`、`type SemNum = u16`、`type SemOp = i16`：semaphore 上下文内部编号。
- `type ShmId = usize`：共享内存上下文内部编号。
- `shm_get_or_create(key, npages, store) -> Arc<Mutex<Vec<usize>>>`：按 key 从 weak store 复用共享页集合，或创建 `npages` 个页槽。

## K7：进程、线程、调度与进程组

### `ProcInit`

职责：构造用户栈初始布局的模拟器。

字段：

- `args: Vec<String>`：argv 字符串。
- `envs: Vec<String>`：环境变量字符串。
- `auxv: BTreeMap<u8, usize>`：auxiliary vector。

方法：

- `push_at(top) -> usize`：从给定栈顶向下估算放置 args/envs/auxv 后的新栈指针，并做 16 字节对齐。
- `total_size() -> usize`：估算初始栈布局所需字节数。

### `Context`

职责：通用寄存器上下文，保存模拟用户/内核切换时的寄存器、指令指针和 flags。

字段：

- `r: [u64; N_REGS]`：通用寄存器数组。
- `ip: u64`：指令指针。
- `flags: u64`：状态标志。

方法：

- `new() -> Self`：创建全 0 上下文。
- `capture(src) -> Self`：从寄存器数组复制上下文。
- `apply() -> [u64; N_REGS]`：导出寄存器数组。
- `set_ip(v)`：设置指令指针。
- `set_sp(v)`：设置最后一个寄存器为栈指针。
- `set_ret(v)`：设置返回值寄存器 `r[0]`。
- `set_tls(v)`：设置倒数第二个寄存器为 TLS。
- `transform(op, val) -> Context`：按 op 生成修改后的上下文，用于测试寄存器变换。
- `syscall_args() -> (u64, u64, u64, u64, u64, u64)`：取前 6 个系统调用参数。
- `clone_with_ret(ret) -> Context`：复制上下文但替换返回值。
- `diff(other) -> Vec<(usize, u64, u64)>`：列出与另一个上下文不同的寄存器/IP/flags。
- `hash() -> u64`：计算上下文哈希。
- `reg_class(idx) -> u64`：按高 4 位分类读取寄存器值，模拟寄存器类别处理。

### `TrapCtl`

职责：陷入/中断控制器模拟，维护 mask、嵌套深度、当前 frame、IRQ 状态和 suppress 状态。

字段：

- `active: AtomicBool`：是否正在处理 trap/irq。
- `hw_mask: AtomicU32`：硬件 vector mask。
- `sw_mask: AtomicU32`：软件 vector mask。
- `nest: AtomicUsize`：嵌套深度。
- `frame: Mutex<Option<Context>>`：当前 trap frame。
- `stack: Mutex<Vec<Context>>`：frame 栈。
- `irq_on: AtomicBool`：IRQ 是否开启。
- `suppressed: AtomicBool`：是否 suppress 中断处理。

方法：

- `new() -> Self`：创建默认控制器。
- `configure(a, b)`：设置软件/硬件 mask。
- `hw() -> u32`：读取硬件 mask。
- `sw() -> u32`：读取软件 mask。
- `in_handler() -> bool`：判断是否处于 handler 或嵌套中。
- `dispatch(ctx) -> Context`：保存当前 frame，短暂增加嵌套深度，并返回上下文副本。
- `current() -> Option<Context>`：读取当前 frame 副本。
- `handle_irq(ctx) -> Context`：模拟 IRQ 进入、保存 frame、处理后退出 active。
- `on_pgfault(va) -> Result<(), &'static str>`：检查 page fault 是否发生在内核地址或 interrupt handler 内。
- `dispatch_vector(vector, ctx) -> Context`：按 vector 和 mask 决定是否 dispatch。
- `push_frame(ctx)`：压入 frame 栈。
- `pop_frame() -> Option<Context>`：弹出 frame。
- `nest_depth() -> usize`：读取嵌套深度。
- `suppress()`：开启 suppress。
- `unsuppress()`：关闭 suppress。

### `SchedulePolicy`

职责：任务调度策略、优先级、nice、时间片和 CFS 风格 vruntime。

字段：

- `policy: u8`：调度策略，例如 `SCHED_NORMAL`。
- `prio: i32`：静态优先级。
- `nice: i32`：nice 值。
- `time_slice: usize`：时间片。
- `vruntime: u64`：虚拟运行时间。

方法：

- `new() -> Self`：默认普通调度、默认优先级、10 tick 时间片。
- `with_prio(prio) -> Self`：用给定优先级创建策略，并派生 nice 和时间片。
- `weight() -> u64`：按 nice 粗略映射 Linux CFS 权重。

### `RunQueue`

职责：运行队列，保存可运行任务和当前任务，支持选择、重排和抢占计数。

字段：

- `queue: Mutex<Vec<(usize, SchedulePolicy)>>`：任务 id 与调度策略列表。
- `current: Mutex<Option<usize>>`：当前运行任务 id。
- `preempt_count: AtomicUsize`：抢占禁用计数。

方法：

- `new() -> Self`：创建空运行队列。
- `enqueue(task_id, policy)`：插入任务并按评分冒泡排序。
- `dequeue() -> Option<(usize, SchedulePolicy)>`：选择评分最小的任务并移除。
- `pick_next() -> Option<usize>`：查看下一个应运行任务但不移除。
- `cmp_priority(a, b) -> CmpOrd`：私有优先级比较函数。
- `rebalance()`：按 tick 和权重推进 vruntime，并按 vruntime 重排。
- `set_current(id)`：设置当前任务。
- `clear_current()`：清空当前任务。
- `len() -> usize`：运行队列长度。
- `remove(task_id) -> bool`：删除指定任务。
- `update_vruntime(task_id, delta)`：按任务权重缩放并增加 vruntime。
- `preempt_disable()`：抢占禁用计数加一。
- `preempt_enable()`：抢占禁用计数减一。
- `preemptible() -> bool`：判断抢占计数是否为 0。
- `boost_priority(task_id, amount)`：提高任务优先级，数值上减少 `prio`。
- `yield_current() -> bool`：把当前任务重新放回队列。

### `Pid`

职责：进程 id 的包装类型。

字段：

- tuple 字段 `pub usize`：实际 pid 数值。

关联常量/方法：

- `INIT: usize = 1`：init 进程 pid。
- `new() -> Self`：创建 pid 0。
- `get() -> usize`：读取 pid。
- `is_init() -> bool`：判断是否 pid 1。

Trait：

- `fmt::Display`：按数字输出 pid。

相关类型别名：

- `type Tid = usize`：线程 id。
- `type Pgid = i32`：进程组 id。

### `TaskInfo`

职责：可展示的任务摘要信息。

字段：

- `id: usize`：任务 id。
- `tag: String`：任务标签或路径。
- `status: Option<i32>`：退出状态；`None` 表示未退出。
- `fds: Vec<String>`：fd 描述字符串，当前主要作为展示占位。

方法：

- 无独立方法。

### `ThdCtx`

职责：线程运行上下文，包含用户上下文、clear tid 和信号 mask。

字段：

- `uctx: Context`：用户寄存器上下文。
- `clear_tid: usize`：线程退出时清理的用户地址。
- `smask: u64`：线程信号 mask。

方法/Trait：

- `Default::default()`：创建默认上下文、clear tid 0、mask 0。

### `Task`

职责：模拟进程/线程实体，是 fd、cwd、exec、futex、IPC、信号、epoll、内核栈和线程上下文的聚合点。

字段：

- `info: Mutex<TaskInfo>`：任务基本信息。
- `parent: Mutex<Option<Arc<Task>>>`：父任务。
- `subtasks: Mutex<Vec<Arc<Task>>>`：子任务列表。
- `files: Mutex<BTreeMap<usize, FLike>>`：fd 表。
- `cwd: Mutex<String>`：当前工作目录。
- `exec_path: Mutex<String>`：当前执行路径。
- `futexes: Mutex<BTreeMap<usize, Arc<FutexBucket>>>`：用户地址到 futex 桶。
- `sem_ctx: Mutex<SemCtx>`：进程 semaphore 上下文。
- `shm_ctx: Mutex<ShmCtx>`：进程共享内存上下文。
- `pid: Mutex<Pid>`：进程 id。
- `pgid: Mutex<Pgid>`：进程组 id。
- `threads: Mutex<Vec<Tid>>`：线程 id 列表。
- `ev: Arc<Mutex<EvBus>>`：任务事件总线。
- `exit_code: Mutex<usize>`：退出码。
- `sig_queue: Mutex<VecDeque<(i32, isize)>>`：信号队列，保存信号号和发送者 tid。
- `sig_mask: Mutex<u64>`：进程信号 mask。
- `ep_inst: Mutex<BTreeMap<usize, EpInst>>`：fd 到 epoll 实例。
- `kstk: Mutex<Option<KStk>>`：内核栈。
- `thd_ctx: Mutex<Option<ThdCtx>>`：可取出的线程上下文。
- `vm_token: AtomicUsize`：模拟地址空间 token 或 brk。

方法：

- `make(id, tag) -> Arc<Self>`：创建新任务对象。
- `id() -> usize`：读取任务 id。
- `tag() -> String`：读取任务标签。
- `link_parent(p)`：设置父任务。
- `link_child(c)`：追加子任务。
- `done() -> bool`：判断是否已有退出状态。
- `n_children() -> usize`：子任务数量。
- `get_free_fd() -> usize`：从 0 起找空闲 fd。
- `get_free_fd_from(arg) -> usize`：从指定 fd 起找空闲 fd。
- `add_file(fl) -> usize`：插入文件对象并返回 fd。
- `get_file(fd) -> Option<FLike>`：读取 fd 对应对象副本。
- `get_futex(uaddr) -> Arc<FutexBucket>`：获取或创建用户地址对应 futex 桶。
- `exit_proc(code)`：关闭 fd，发布退出/子退出事件，保存退出码并清空线程。
- `exited() -> bool`：判断线程为空或已有退出状态。
- `get_ep_mut(fd) -> Result<EpInst, &'static str>`：获取 epoll 实例副本。
- `get_ep_ref(fd) -> Result<EpInst, &'static str>`：当前等同 `get_ep_mut`。
- `set_ep(fd, inst)`：设置 epoll 实例。
- `begin_run() -> ThdCtx`：取出线程上下文；没有则返回默认上下文。
- `end_run(cx)`：放回线程上下文。
- `has_sig() -> bool`：检查是否存在未被 mask 且匹配当前线程的信号。
- `send_sig(signo, sender_tid)`：入队信号并发布 `RECV_SIG`。
- `close_fd(fd) -> Result<(), &'static str>`：关闭 fd。
- `dup_fd(old_fd, cloexec) -> Result<usize, &'static str>`：复制 fd 到最小空闲 fd。
- `dup2_fd(old_fd, new_fd) -> Result<usize, &'static str>`：复制 fd 到指定新 fd。
- `fd_count() -> usize`：fd 表条目数。
- `set_cloexec(fd, val) -> Result<(), &'static str>`：检查 fd 存在；当前没有真实改写底层 cloexec。

Trait：

- `fmt::Debug`：输出任务 id 和 tag。

### `TaskTable`

职责：全局任务表，负责创建、查找、fork/clone、回收和进程组信号。

字段：

- `map: RwLock<BTreeMap<usize, Arc<Task>>>`：任务 id 到任务对象。
- `seq: AtomicUsize`：下一个任务 id。
- `root: Mutex<Option<Arc<Task>>>`：init/root 任务。

方法：

- `new() -> Self`：创建空任务表。
- `spawn(tag) -> Arc<Task>`：分配新 id 并插入任务表。
- `spawn_root() -> Arc<Task>`：创建 init 任务并保存为 root。
- `find(id) -> Option<Arc<Task>>`：按 id 查找任务。
- `find_by_tag(tag) -> Vec<Arc<Task>>`：按 tag 查找任务。
- `process_of_tid(tid) -> Option<Arc<Task>>`：查找包含指定线程 id 的进程。
- `pgid_group(pgid) -> Vec<Arc<Task>>`：列出进程组内任务。
- `register(task, pid)`：设置任务 pid 并按 pid 插入任务表。
- `reap(id)`：回收任务，把其子任务交给 root，并从 map 移除。
- `count() -> usize`：任务表大小。
- `fork_task(src) -> Arc<Task>`：复制任务的 cwd、exec_path、fd、IPC 上下文、信号 mask、进程组等。
- `clone_thread(src, stack_top, tls, clear_tid) -> Arc<Task>`：创建同地址空间线程并设置 SP/TLS/clear_tid。
- `new_user_task(path, args, envs) -> Arc<Task>`：创建用户任务，设置 exec path、初始用户栈和标准 fd。
- `terminate_and_collect(id, code) -> bool`：退出并回收指定任务。
- `active_tasks() -> Vec<usize>`：列出未退出任务 id。
- `zombie_tasks() -> Vec<usize>`：列出已退出任务 id。
- `send_signal_group(pgid, signo) -> usize`：向进程组发送信号，返回发送数量。

### `ProcessGroup`

职责：进程组和会话模拟。

字段：

- `pgid: Pgid`：进程组 id。
- `leader: usize`：组长 pid。
- `members: Mutex<Vec<usize>>`：成员 pid 列表。
- `session_id: usize`：所属 session id。
- `foreground: AtomicBool`：是否前台进程组。

方法：

- `new(pgid, leader, session) -> Self`：创建进程组，初始成员包含 leader。
- `add_member(pid)`：添加成员，避免重复。
- `remove_member(pid) -> bool`：删除成员。
- `is_empty() -> bool`：判断是否无成员。
- `member_count() -> usize`：成员数量。
- `is_leader(pid) -> bool`：判断 pid 是否组长。
- `set_foreground(fg)`：设置前台状态。
- `is_foreground() -> bool`：读取前台状态。
- `broadcast_signal(signo, tasks)`：向所有成员任务发送信号。

### `ResourceLimits`

职责：进程资源限制对象。

字段：

- `max_fds: usize`：最大 fd 数。
- `max_threads: usize`：最大线程数。
- `max_stack_size: usize`：最大栈大小。
- `max_data_size: usize`：最大数据段大小。
- `max_file_size: usize`：最大文件大小。
- `max_mappings: usize`：最大映射数量。
- `cpu_time_limit: usize`：CPU 时间限制。

方法：

- `default_limits() -> Self`：返回默认资源限制。
- `check_fd(current) -> bool`：判断 fd 数是否未超限。
- `check_threads(current) -> bool`：判断线程数是否未超限。
- `check_stack(requested) -> bool`：判断栈请求是否未超限。
- `check_data(requested) -> bool`：判断数据段请求是否未超限。
- `check_filesize(requested) -> bool`：判断文件大小是否未超限。
- `check_mappings(current) -> bool`：判断映射数量是否未超限。
- `inherit() -> Self`：复制当前限制。
- `set_limit(resource, value) -> Result<(), &'static str>`：按资源编号设置限制。
- `get_limit(resource) -> Result<usize, &'static str>`：按资源编号读取限制。
- `exceeds_any(fds, threads, stack) -> bool`：检查 fd、线程、栈三类是否任一超限。

相关自由函数：

- `yield_now_sync()`：调用 `thread::yield_now()`，用于同步让出 CPU。
- `compute_load_balance(task_counts, priorities, io_blocked) -> usize`：根据负载、优先级和阻塞状态选择目标 CPU。虽然在 K11 也可视为算法函数，但它直接服务调度层。

## K8：权限、信号与时间

### `CapSet`

职责：Linux capability 风格的权限位集合。

字段：

- `bits: u64`：permitted capability 位。
- `effective: u64`：effective capability 位。
- `ambient: u64`：ambient capability 位。

方法：

- `new() -> Self`：创建空 capability 集。
- `full() -> Self`：创建 permitted/effective 全 1 的集合。
- `check(cap) -> bool`：检查 effective 中是否包含某能力。
- `grant(cap)`：授予并启用某能力。
- `drop_cap(cap)`：删除某能力。
- `inherit(parent) -> CapSet`：按 `INHERITABLE_MASK` 从父集合继承能力。
- `has_any(mask) -> bool`：检查 effective 是否包含 mask 中任意位。
- `clear_ambient()`：清空 ambient。
- `raise_ambient(cap) -> bool`：当 permitted 中有该能力时提升到 ambient。

### `SigAction`

职责：单个信号的处理动作。

字段：

- `handler: usize`：handler 地址或 `SIG_DFL/SIG_IGN`。
- `flags: u32`：动作标志。
- `mask: u64`：执行 handler 时附加阻塞的信号 mask。

方法：

- 无独立方法；由 `SigSet` 管理。

### `SigSet`

职责：信号 pending/blocked 集合和每个信号的动作表。

字段：

- `pending: u64`：待处理信号位。
- `blocked: u64`：被阻塞信号位。
- `actions: Vec<SigAction>`：信号号到动作。

方法：

- `new() -> Self`：创建空 pending/blocked，并为 0..=NSIG 初始化默认动作。
- `sig_pending(signo) -> bool`：检查某信号是否 pending。
- `sig_raise(signo)`：把信号置为 pending。
- `coalesce_pending() -> u64`：返回 pending 且未 blocked 的信号集合。
- `sig_clear(signo)`：清除 pending。
- `sig_block(mask)`：阻塞 mask 中信号，但不能阻塞 `SIGKILL/SIGSTOP`。
- `sig_unblock(mask)`：解除阻塞。
- `sig_setmask(mask)`：直接设置阻塞集合，同样排除 `SIGKILL/SIGSTOP`。
- `deliverable() -> Option<u32>`：返回第一个可递送信号。
- `set_action(signo, action)`：设置信号动作，禁止修改 `SIGKILL/SIGSTOP`。
- `get_action(signo) -> &SigAction`：读取信号动作，越界返回 actions[0]。
- `is_ignored(signo) -> bool`：判断 handler 是否 `SIG_IGN`。
- `clear_non_caught()`：exec 后把非默认、非忽略 handler 恢复成默认。

### `TimerEntry`

职责：定时器条目。

字段：

- `deadline: usize`：到期 tick。
- `interval: usize`：周期 tick；0 表示一次性。
- `callback_id: usize`：回调 id。
- `active: bool`：是否有效。
- `repeat: bool`：是否周期定时器。

方法：

- `new(deadline, interval, cb_id) -> Self`：创建定时器，`interval > 0` 时 repeat。
- `expired() -> bool`：判断 `CLK` 是否超过 deadline。
- `reset()`：周期定时器设置下次 deadline，一次性定时器则停用。
- `remaining() -> usize`：返回距离到期的剩余 tick。
- `cancel()`：停用定时器。

### `TimerWheel`

职责：时间轮，按 deadline modulo 分桶存放 `TimerEntry`。

字段：

- `slots: Vec<Vec<TimerEntry>>`：时间轮槽。
- `current_slot: usize`：当前槽索引。

方法：

- `new() -> Self`：创建 `TIMER_WHEEL_SIZE` 个空槽。
- `add_timer(entry)`：按 `entry.deadline % TIMER_WHEEL_SIZE` 加入槽。
- `advance() -> Vec<TimerEntry>`：推进当前槽，取出已到期定时器；周期定时器会重排到新槽。
- `cancel(cb_id) -> bool`：按 callback id 停用定时器。
- `active_count() -> usize`：统计 active 定时器数量。

相关静态对象/自由函数：

- `CLK: AtomicUsize`：CPU0 推进的主 tick。
- `CLK_ALL: AtomicUsize`：所有 CPU tick 汇总。
- `wclk() -> usize`：读取 `CLK`。
- `cclk() -> usize`：读取 `CLK_ALL`。
- `dtk(cpu_id)`：CPU0 推进 `CLK`，所有 CPU 推进 `CLK_ALL`。
- `up_ms() -> usize`：把 tick 转换成毫秒。
- `tmr(cpu_id)`：调用 `dtk(cpu_id)`。

## K9：网络与协议工具

这一层在 `kernel.rs` 里主要是枚举和自由函数，没有复杂结构体。

### `SocketState`

职责：TCP socket 状态枚举。

变体：

- `Closed`：关闭。
- `Listen`：监听。
- `SynSent`：已发 SYN。
- `SynRecvd`：已收到 SYN。
- `Established`：连接建立。
- `FinWait1`：主动关闭第一阶段。
- `FinWait2`：主动关闭第二阶段。
- `TimeWait`：TIME_WAIT。
- `CloseWait`：被动关闭等待本地关闭。
- `LastAck`：等待最后 ACK。
- `Closing`：双方同时关闭。

相关自由函数：

- `tcp_checksum(src_ip, dst_ip, payload) -> u16`：带 TCP pseudo-header 的 checksum。
- `parse_ipv4_header(pkt) -> Option<(u32, u32, u8, u16)>`：解析 IPv4 头，返回源地址、目的地址、协议号和总长度。
- `build_pseudo_header(src, dst, proto, length) -> Vec<u8>`：构造 TCP/UDP checksum pseudo-header。
- `compute_inet_checksum(data) -> u16`：通用 Internet checksum。

## K10：系统调用与 `Kernel` 门面

### `Kernel`

职责：模拟内核顶层对象，聚合任务表、块缓存、页帧池、CPU 当前任务、挂载表、IPC 全局 weak store、TTY 缓冲和磁盘。

字段：

- `tasks: TaskTable`：全局任务表。
- `cache: BlockCache`：块缓存。
- `pool: FramePool`：页帧池。
- `cpus: Mutex<[Option<Arc<Task>>; MAX_CPU]>`：每个 CPU 当前任务。
- `mnt: MountTable`：挂载表。
- `sem_store: RwLock<BTreeMap<u32, Weak<SemArr>>>`：System V semaphore 全局 weak store。
- `shm_store: RwLock<BTreeMap<usize, Weak<Mutex<Vec<usize>>>>>`：共享内存全局 weak store。
- `tty_buf: Mutex<VecDeque<u8>>`：TTY 输入缓冲。
- `disk: Disk`：主磁盘模拟对象。

生命周期/调度方法：

- `new(nf) -> Self`：初始化所有内核子系统，页帧池大小为 `nf`。
- `tick(id)`：进入 `GKL`，扫描 CPU 占用并清理块缓存 dirty 标记。
- `schedule_tick(cpu)`：推进 tick，并根据当前任务估算是否需要调度。
- `balance_load() -> usize`：收集 CPU 负载并调用 `compute_load_balance` 选择目标 CPU。
- `reclaim_zombies() -> usize`：回收 zombie 任务，返回回收数量。

当前任务/进程方法：

- `cur_task(cpu) -> Option<Arc<Task>>`：读取某 CPU 当前任务。
- `set_cur(cpu, t)`：设置某 CPU 当前任务。
- `proc_init()`：创建 init 任务并为其设置内核栈。
- `spawn_thread(task) -> thread::JoinHandle<()>`：为任务创建宿主线程，循环 begin/end run 直到任务退出。
- `do_fork(parent_id) -> Result<usize, &'static str>`：以任务表 fork 父任务，复制 vm token 并返回子 id。
- `do_exec(task_id, path, args, envs) -> Result<(), &'static str>`：更新 exec path，关闭 cloexec fd，重建初始用户上下文。
- `do_pipe(task_id) -> Result<(usize, usize), &'static str>`：为任务创建管道并返回读写 fd。
- `do_wait(parent_id, target_pid, options) -> Result<(usize, usize), &'static str>`：按 wait4 规则寻找 zombie 子进程并回收。

内存/路径方法：

- `handle_pgfault(addr) -> bool`：模拟 page fault 处理，当前要求 CPU0 有当前任务。
- `handle_pgfault_ext(addr, access) -> bool`：带 access 类型的 page fault 入口，当前转发到 `handle_pgfault`。
- `lookup_path(path) -> Result<String, &'static str>`：规整路径并用挂载表解析。
- `alloc_pages(count) -> Vec<usize>`：分配多个页地址。
- `free_pages(pages)`：释放多个页地址。
- `memory_pressure() -> usize`：按页帧池使用率返回内存压力百分比。
- `cache_stats() -> (usize, usize)`：返回块缓存条目数和脏块数。

TTY 与 IPC 方法：

- `tty_push(c)`：把输入字节规范化后推入 TTY 缓冲。
- `tty_pop() -> Option<u8>`：弹出 TTY 字节。
- `get_sem(key, nsems, flags) -> Result<Arc<SemArr>, &'static str>`：获取或创建 semaphore array。
- `get_shm(key, npages) -> Arc<Mutex<Vec<usize>>>`：获取或创建共享内存页集合。

系统调用分发：

- `dispatch_syscall(nr, a0, a1, a2, a3, a4, a5) -> Result<usize, &'static str>`：测试用 ABI 适配层，根据 `SYS_*` 号模拟系统调用行为。
  - 文件 I/O：`SYS_READ`、`SYS_WRITE`、`SYS_OPEN`、`SYS_CLOSE`、`SYS_STAT`、`SYS_FSTAT`、`SYS_FCNTL`。
  - 内存：`SYS_MMAP`、`SYS_MUNMAP`、`SYS_BRK`。
  - 终端/管道/fd：`SYS_IOCTL`、`SYS_PIPE`、`SYS_DUP`、`SYS_DUP2`。
  - 进程：`SYS_FORK`、`SYS_EXEC`、`SYS_EXIT`、`SYS_WAIT4`、`SYS_GETPID`、`SYS_GETPPID`、`SYS_SETPGID`、`SYS_GETPGID`、`SYS_SETSID`。
  - 信号/时间/epoll/futex：`SYS_KILL`、`SYS_EPOLL_CREATE`、`SYS_EPOLL_CTL`、`SYS_EPOLL_WAIT`、`SYS_CLOCK_GETTIME`、`SYS_SIGACTION`、`SYS_SIGPROCMASK`、`SYS_FUTEX`。
  - 主要职责是参数检查、地址合法性检查、状态模拟和返回值构造；它不真正复制用户内存。

## K11：通用算法和诊断函数

这一层主要是自由函数。它没有专属结构体，但很多函数服务前面各层的测试和诊断。

### ELF、负载与 fd/挂载诊断

- `validate_elf_header(data) -> Result<usize, &'static str>`：检查 ELF magic、64 位、小端、版本、类型、program header 范围和 LOAD 段数量，成功返回入口地址。
- `compute_load_balance(task_counts, priorities, io_blocked) -> usize`：给 CPU 打分并选择迁移目标；也被 K7/K10 调度逻辑调用。
- `audit_fd_table(files) -> Vec<usize>`：扫描 fd 表空洞、异常 pipe 和空路径文件。
- `rehash_mount_cache(entries) -> BTreeMap<u64, usize>`：为挂载项生成哈希到索引的缓存映射。

### 内存诊断与访问检查

- `defragment_frame_pool(slots) -> usize`：扫描 bool slot，统计空闲页和碎片信息，当前返回空闲数量。
- `verify_page_alignment(addr, order) -> bool`：检查地址是否按 `PAGE_SZ << order` 对齐，且 order/范围有效。
- `compute_rss_watermark(regions, pool_cap) -> usize`：按区域权限和 shared/private 属性估算 RSS 水位。
- `validate_access(mode, addr, len, pid) -> Result<(), &'static str>`：更完整的用户地址访问检查，区分普通、读、写/跨页模式。

### 字节扫描、校验和与编码

- `mem_scan_pattern(data, pattern, max_matches) -> Vec<usize>`：KMP 模式匹配，返回最多 `max_matches` 个匹配偏移。
- `compute_crc32(data) -> u32`：计算 CRC32。
- `encode_varint(value, out) -> usize`：把 u64 编码为 varint，返回写入字节数。
- `decode_varint(data) -> Option<(u64, usize)>`：解码 varint，返回值和消耗字节数。

### 位运算、对齐、order 和哈希

- `bitwise_merge(a, b, mask) -> u64`：按 mask 从 `b` 取位、其余从 `a` 取位。
- `rotate_bits(value, amount, width) -> u64`：在指定 bit 宽度内循环左移。
- `popcount64(v) -> u32`：计算 64 位整数中 1 的数量。
- `clz64(v) -> u32`：计算 leading zeros。
- `ffs64(v) -> Option<u32>`：返回最低 set bit 的位置；0 返回 `None`。
- `align_up(addr, align) -> usize`：按 2 的幂对齐向上取整；非法 align 时返回原地址。
- `align_down(addr, align) -> usize`：按 2 的幂对齐向下取整；非法 align 时返回原地址。
- `is_power_of_two(v) -> bool`：判断是否 2 的幂。
- `log2_floor(v) -> usize`：返回 floor(log2(v))，0 返回 0。
- `order_for_pages(pages) -> usize`：返回能覆盖 `pages` 的 buddy order。
- `hash_combine(seed, value) -> u64`：组合哈希值。
- `murmurhash3_finalize(h) -> u64`：MurmurHash3 finalizer。
