// 本文件是 kernel/src/kernel.rs 的中文注释阅读版。
//
// 生成目的：帮助读者快速理解每个结构体、函数/方法和全局常量的含义。
// 原始可编译文件仍然是 kernel/src/kernel.rs；本文件放在 docs/ 下，仅用于阅读，不参与测试编译。
// 注释策略：为每个 struct、函数/方法和 pub const 全局常量添加简短中文说明。
//
#![allow(
    unused,
    dead_code,
    non_upper_case_globals,
    non_camel_case_types,
    unused_assignments,
    unused_mut
)]

use std::any::Any;
use std::cmp::{max, min, Ordering as CmpOrd};
use std::collections::{BTreeMap, BTreeSet, HashMap, LinkedList, VecDeque};
use std::fmt;
use std::ops::{Deref, DerefMut, Index};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};
use std::thread;
use std::time::Duration;

// ==========================================
// 1. 内存与基础系统限制
// ==========================================
pub const PAGE_SZ: usize = 4096;           // 页面大小 (4KB)。用于 SYS_MMAP, SYS_BRK, SYS_READ/WRITE 的页对齐计算、缓存分页。
pub const N_PROC: usize = 256;             // 系统最大进程数。用于限制 SYS_FORK, SYS_PIPE (限制 fd 数量), SYS_CLOSE/DUP 的边界检查 (fd < N_PROC * 4)。
pub const N_FRAMES: usize = 65536;         // 物理内存总页帧数 (65536 * 4KB = 256MB)。用于物理页帧分配器 (self.pool) 和 SYS_FORK 的内存压力检查。
pub const KERN_BASE: usize = 0xFFFF_FFFF_8000_0000; // 内核虚拟地址基址 (x86-64 高半区)。用于 SYS_BRK 和 SYS_MMAP 防止用户内存越界覆盖内核空间。
pub const PHYS_OFF: usize = 0xFFFF_FFFF_0000_0000;  // 物理内存直接映射区的偏移量。内核用于将物理地址转换为内核虚拟地址。
pub const MEM_OFF: usize = 0x8000_0000;    // 用户态内存或 legacy 内存偏移基址。
pub const KHEAP_SZ: usize = 0x800000;      // 内核堆大小 (8MB)。用于内核内部的 Slab 分配器或 kmalloc。
pub const N_CHAINS: usize = 64;            // 哈希链表桶数量。用于文件缓存结构 self.cache.width，在 SYS_OPEN/READ/WRITE/CLOSE 中通过 fd % width 定位缓存链。
pub const RBUF_CAP: usize = 256;           // 环形缓冲区容量。用于管道 或 TTY 设备的读写缓冲区。
pub const N_REGS: usize = 16;              // CPU 通用寄存器数量。用于 Trap 上下文 结构体，保存/恢复用户态寄存器现场。
pub const MNT_DEPTH: usize = 8;            // 挂载表最大深度。用于 SYS_OPEN 解析路径时的挂载点匹配 (self.mnt.entries)。
pub const MAX_CPU: usize = 8;              // 最大 CPU 核心数。用于 self.cpus 数组初始化及调度器 per-CPU 变量。
pub const KSTK_SZ: usize = 0x4000;         // 内核栈大小 (16KB)。每个进程在内核态执行 syscall 时的独立栈空间。
pub const USR_STK_OFF: usize = 0x7FFF_0000; // 用户态栈顶初始偏移。用于 SYS_EXEC 初始化新进程时的栈基址设定。
pub const USR_STK_SZ: usize = 0x10000;     // 用户态栈大小 (64KB)。
pub const USEC_TICK: usize = 1000;         // 时钟微秒级滴答。用于全局时钟 CLK 的推进和时间戳计算。
pub const FOLLOW_LIM: usize = 3;           // 符号链接跟随最大深度。用于 SYS_OPEN 解析路径时的防死循环保护。

// ==========================================
// 2. 文件描述符与 Open/Fcntl 标志位
// ==========================================
pub const F_DUPFD: usize = 0;              // fcntl 命令：复制文件描述符，寻找 >= arg 的最小可用 fd。用于 SYS_FCNTL。
pub const F_GETFD: usize = 1;              // fcntl 命令：获取 close-on-exec 标志。用于 SYS_FCNTL。
pub const F_SETFD: usize = 2;              // fcntl 命令：设置 close-on-exec 标志。用于 SYS_FCNTL。
pub const F_GETFL: usize = 3;              // fcntl 命令：获取文件访问状态标志 (如 O_NONBLOCK)。用于 SYS_FCNTL。
pub const F_SETFL: usize = 4;              // fcntl 命令：设置文件访问状态标志。用于 SYS_FCNTL。
pub const F_GETLK: usize = 5;              // fcntl 命令：获取文件锁。用于 SYS_FCNTL。
pub const F_SETLK: usize = 6;              // fcntl 命令：设置文件锁 (非阻塞)。用于 SYS_FCNTL。
pub const F_SETLKW: usize = 7;             // fcntl 命令：设置文件锁 (阻塞)。用于 SYS_FCNTL。
pub const FD_CLOEXEC: usize = 1;           // close-on-exec 标志位的具体值。用于 SYS_FCNTL 的 F_GETFD/F_SETFD 判断。
pub const F_DUPFD_CLOEXEC: usize = 1030;   // fcntl 命令：复制 fd 并设置 close-on-exec。用于 SYS_FCNTL。
pub const O_NONBLOCK: usize = 0o4000;      // open/fcntl 标志：非阻塞 I/O。用于 SYS_OPEN 和 SYS_PIPE 的 flags 解析。
pub const O_APPEND: usize = 0o2000;        // open/fcntl 标志：每次写操作前将文件指针移到末尾。用于 SYS_OPEN 和 F_GETFL/F_SETFL。
pub const O_CLOEXEC: usize = 0o2000000;    // open 标志：执行 exec 时自动关闭 fd。用于 SYS_OPEN 和 SYS_PIPE。
pub const AT_NOFOLLOW: usize = 0x100;      // open 标志：不解析符号链接。用于 SYS_OPEN 的 _follow_sym 逻辑。

// ==========================================
// 3. 终端 IOCTL 命令
// ==========================================
pub const TCGETS: usize = 0x5401;          // ioctl 命令：获取终端属性。用于 SYS_IOCTL 校验 TrmIO 结构体大小。
pub const TCSETS: usize = 0x5402;          // ioctl 命令：设置终端属性。用于 SYS_IOCTL 校验。
pub const TIOCGPGRP: usize = 0x540F;       // ioctl 命令：获取前台进程组 ID。用于 SYS_IOCTL。
pub const TIOCSPGRP: usize = 0x5410;       // ioctl 命令：设置前台进程组 ID。用于 SYS_IOCTL。
pub const TIOCGWINSZ: usize = 0x5413;      // ioctl 命令：获取终端窗口大小。用于 SYS_IOCTL 校验 WinSz 结构体。
pub const FIONCLEX: usize = 0x5450;        // ioctl 命令：清除 close-on-exec 标志。用于 SYS_IOCTL。
pub const FIOCLEX: usize = 0x5451;         // ioctl 命令：设置 close-on-exec 标志。用于 SYS_IOCTL。
pub const FIONBIO: usize = 0x5421;         // ioctl 命令：设置/清除非阻塞 I/O 模式。用于 SYS_IOCTL。

// ==========================================
// 4. ELF 辅助向量
// ==========================================
pub const AT_PHDR: u8 = 3;                 // 辅助向量：程序头表地址。用于 SYS_EXEC 构造新进程栈底信息传给动态链接器。
pub const AT_PHENT: u8 = 4;                // 辅助向量：程序头条目大小。用于 SYS_EXEC。
pub const AT_PHNUM: u8 = 5;                // 辅助向量：程序头条目数量。用于 SYS_EXEC。
pub const AT_PAGESZ: u8 = 6;               // 辅助向量：系统页大小。用于 SYS_EXEC。
pub const AT_BASE: u8 = 7;                 // 辅助向量：解释器 (ld.so) 基址。用于 SYS_EXEC。
pub const AT_ENTRY: u8 = 9;                // 辅助向量：程序入口点地址。用于 SYS_EXEC。

// ==========================================
// 5. 终端线路规范 标志位
// ==========================================
pub const LM_ISIG: u32 = 0o000001;         // termios c_lflag：产生信号 (如 Ctrl+C 产生 SIGINT)。用于 TrmIO 结构体。
pub const LM_ICANON: u32 = 0o000002;       // termios c_lflag：规范模式 (行缓冲)。
pub const LM_ECHO: u32 = 0o000010;         // termios c_lflag：回显输入字符。
pub const LM_ECHOE: u32 = 0o000020;        // termios c_lflag：擦除时回显退格。
pub const LM_ECHOK: u32 = 0o000040;        // termios c_lflag：Kill 行时回显换行。
pub const LM_ECHONL: u32 = 0o000100;       // termios c_lflag：回显 NL 即使没有 ECHO。
pub const LM_NOFLSH: u32 = 0o000200;       // termios c_lflag：中断时不刷新输入输出队列。
pub const LM_TOSTOP: u32 = 0o000400;       // termios c_lflag：后台写操作发送 SIGTTOU。
pub const LM_IEXTEN: u32 = 0o100000;       // termios c_lflag：启用实现自定义的输入处理。
pub const LM_XCASE: u32 = 0o000004;        // termios c_lflag：大小写映射 (已废弃)。
pub const LM_ECHOCTL: u32 = 0o001000;      // termios c_lflag：回显控制字符为 ^X。
pub const LM_ECHOPRT: u32 = 0o002000;      // termios c_lflag：硬拷贝擦除模式。
pub const LM_ECHOKE: u32 = 0o004000;       // termios c_lflag：Kill 行时执行擦除操作。
pub const LM_FLUSHO: u32 = 0o010000;       // termios c_lflag：输出被刷新。
pub const LM_PENDIN: u32 = 0o040000;       // termios c_lflag：输入挂起 (下次读时重新处理)。
pub const LM_EXTPROC: u32 = 0o200000;      // termios c_lflag：外部处理模式 (旁路 termios)。

// ==========================================
// 6. 虚拟内存区域 (VMA) 标志位
// ==========================================
pub const VM_READ: u32 = 0x01;             // VMA 权限：可读。用于 SYS_MMAP 转换 prot 参数。
pub const VM_WRITE: u32 = 0x02;            // VMA 权限：可写。用于 SYS_MMAP 转换 prot 参数。
pub const VM_EXEC: u32 = 0x04;             // VMA 权限：可执行。用于 SYS_MMAP 转换 prot 参数。
pub const VM_SHARED: u32 = 0x08;           // VMA 属性：共享映射。用于 SYS_MMAP 判断 MAP_SHARED。
pub const VM_GROWSDOWN: u32 = 0x10;        // VMA 属性：向下生长 (如用户栈)。
pub const VM_DONTCOPY: u32 = 0x20;         // VMA 属性：fork 时不复制 (如内核专属区)。
pub const VM_HUGETLB: u32 = 0x40;          // VMA 属性：使用大页。
pub const VM_PFNMAP: u32 = 0x80;           // VMA 属性：页帧映射 (如设备内存映射)。

// ==========================================
// 7. Linux Capabilities (特权能力)
// ==========================================
pub const CAP_CHOWN: u32 = 0;              // 允许改变文件所有者。用于 SYS_OPEN 等权限检查。
pub const CAP_KILL: u32 = 5;               // 允许向不属于自己用户的进程发信号。用于 SYS_KILL。
pub const CAP_SETUID: u32 = 7;             // 允许设置进程 UID。
pub const CAP_SETGID: u32 = 6;             // 允许设置进程 GID。
pub const CAP_NET_BIND: u32 = 10;          // 允许绑定 1024 以下端口。
pub const CAP_NET_RAW: u32 = 13;           // 允许使用原始套接字。
pub const CAP_SYS_ADMIN: u32 = 21;         // 超级管理员权限 (mount, 设置hostname等)。
pub const CAP_SYS_PTRACE: u32 = 19;        // 允许 ptrace 其他进程。
pub const INHERITABLE_MASK: u64 = 0x0000_00FF_FFFF_FFFF; // 可继承的能力位掩码，用于 SYS_FORK。

// ==========================================
// 8. 物理内存分区
// ==========================================
pub const ZONE_DMA: usize = 0;             // DMA 内存区 (<16MB)，供老式 ISA 设备使用。用于 self.pool 分区分配。
pub const ZONE_NORMAL: usize = 1;          // 正常内存区，内核和用户态主要使用区。
pub const ZONE_HIGH: usize = 2;            // 高端内存区 (32位系统特有，大于 1GB 物理内存部分)。
pub const N_ZONES: usize = 3;              // 内存区数量。

// ==========================================
// 9. 进程调度
// ==========================================
pub const PRIO_MIN: i32 = -20;             // 进程优先级最小值 (最高优先级)。用于 Task 优先级计算。
pub const PRIO_MAX: i32 = 19;              // 进程优先级最大值 (最低优先级)。
pub const PRIO_DEFAULT: i32 = 0;           // 默认优先级。
pub const SCHED_NORMAL: u8 = 0;            // 调度策略：普通时间片轮转。
pub const SCHED_FIFO: u8 = 1;              // 调度策略：实时先进先出。
pub const SCHED_RR: u8 = 2;                // 调度策略：实时时间片轮转。
pub const SCHED_BATCH: u8 = 3;             // 调度策略：批处理模式。

// ==========================================
// 10. 内核 Slab 分配器
// ==========================================
pub const SLAB_OBJ_MIN: usize = 8;         // Slab 分配器最小对象大小 (8字节)。
pub const SLAB_OBJ_MAX: usize = 2048;      // Slab 分配器最大对象大小 (2KB)。
pub const SLAB_ALIGN: usize = 8;           // Slab 对象对齐字节数。

// ==========================================
// 11. 信号
// ==========================================
pub const NSIG: u32 = 64;                  // 最大信号数。用于 SYS_KILL 验证 sig <= NSIG。
pub const SIG_DFL: usize = 0;              // 默认信号处理函数。用于 SYS_SIGACTION 未设置时的处理。
pub const SIG_IGN: usize = 1;              // 忽略信号的处理函数。
pub const SIGKILL: u32 = 9;                // 强制终止进程信号。用于 SYS_KILL 的特殊检查 (不允许发给 PID<=1)。
pub const SIGSTOP: u32 = 19;               // 暂停进程信号。用于 SYS_KILL 的特殊检查。
pub const SIGCHLD: u32 = 17;               // 子进程状态改变信号。用于 SYS_EXIT 通知父进程。
pub const SIGUSR1: u32 = 10;               // 用户自定义信号 1。
pub const SIGUSR2: u32 = 12;               // 用户自定义信号 2。
pub const SIGALRM: u32 = 14;               // 定时器超时信号。

// ==========================================
// 12. 定时器
// ==========================================
pub const TIMER_WHEEL_SIZE: usize = 256;   // 定时器轮大小。用于内核定时器管理结构。
pub const TIMER_TICK_HZ: usize = 100;      // 时钟中断频率 (100Hz，每 10ms 一次)。

// ==========================================
// 13. 网络套接字
// ==========================================
pub const SOCK_STREAM: u32 = 1;            // TCP 流式套接字。用于 socket 相关 syscall (本模拟器中未展开实现)。
pub const SOCK_DGRAM: u32 = 2;             // UDP 数据报套接字。
pub const SOCK_RAW: u32 = 3;               // 原始套接字。
pub const AF_INET: u32 = 2;                // IPv4 地址族。
pub const AF_INET6: u32 = 10;              // IPv6 地址族。
pub const AF_UNIX: u32 = 1;                // Unix 域套接字 (本地进程通信)。

// ==========================================
// 14. 系统调用号 (x86_64 Linux 标准)
// ==========================================
pub const SYS_READ: usize = 0;             // 读文件。用于 dispatch_syscall 的 match 分发。
pub const SYS_WRITE: usize = 1;            // 写文件。用于 dispatch_syscall 的 match 分发。
pub const SYS_OPEN: usize = 2;             // 打开文件。用于 dispatch_syscall。
pub const SYS_CLOSE: usize = 3;            // 关闭文件描述符。用于 dispatch_syscall。
pub const SYS_STAT: usize = 4;             // 获取文件状态 (通过路径)。用于 dispatch_syscall。
pub const SYS_FSTAT: usize = 5;            // 获取文件状态 (通过 fd)。用于 dispatch_syscall。
pub const SYS_MMAP: usize = 9;             // 内存映射。用于 dispatch_syscall。
pub const SYS_MUNMAP: usize = 11;          // 解除内存映射。用于 dispatch_syscall。
pub const SYS_BRK: usize = 12;             // 调整堆空间大小。用于 dispatch_syscall。
pub const SYS_IOCTL: usize = 16;           // 设备控制。用于 dispatch_syscall。
pub const SYS_PIPE: usize = 22;            // 创建管道。用于 dispatch_syscall。
pub const SYS_DUP: usize = 32;             // 复制文件描述符 (最小可用)。用于 dispatch_syscall。
pub const SYS_DUP2: usize = 33;            // 复制文件描述符 (指定目标)。用于 dispatch_syscall。
pub const SYS_FORK: usize = 57;            // 创建子进程。用于 dispatch_syscall。
pub const SYS_EXEC: usize = 59;            // 执行新程序。用于 dispatch_syscall。
pub const SYS_EXIT: usize = 60;            // 退出进程。用于 dispatch_syscall。
pub const SYS_WAIT4: usize = 61;           // 等待子进程状态改变。用于 dispatch_syscall。
pub const SYS_KILL: usize = 62;            // 发送信号。用于 dispatch_syscall。
pub const SYS_FCNTL: usize = 72;           // 文件描述符控制。用于 dispatch_syscall。
pub const SYS_GETPID: usize = 39;          // 获取进程 PID。用于 dispatch_syscall。
pub const SYS_GETPPID: usize = 110;        // 获取父进程 PID。用于 dispatch_syscall。
pub const SYS_SETPGID: usize = 109;        // 设置进程组 ID。用于 dispatch_syscall。
pub const SYS_GETPGID: usize = 121;        // 获取进程组 ID。
pub const SYS_SETSID: usize = 112;         // 创建新会话。
pub const SYS_EPOLL_CREATE: usize = 213;   // 创建 epoll 实例 (I/O多路复用)。
pub const SYS_EPOLL_CTL: usize = 233;      // 操作 epoll 实例 (添加/修改/删除 fd)。
pub const SYS_EPOLL_WAIT: usize = 232;     // 等待 epoll 事件。
pub const SYS_CLOCK_GETTIME: usize = 228;  // 获取高精度时间。
pub const SYS_SIGACTION: usize = 13;       // 设置信号处理函数。
pub const SYS_SIGPROCMASK: usize = 14;     // 设置信号屏蔽字。
pub const SYS_FUTEX: usize = 202;          // 快速用户态互斥锁 (用于自旋锁和条件变量的内核态睡眠唤醒)。

// ==========================================
// 15. 杂项
// ==========================================
pub const BOOT_EPOCH: usize = 0;           // 启动时间纪元 (0 代表从 0 秒开始)。用于 CLK 全局时钟初始化。
pub const IOQUEUE_DEPTH: usize = 128;      // I/O 队列深度 (异步 I/O 如 io_uring 使用的环形队列大小)。用于 self.disk 结构。

/// 虚拟内存区域。
pub struct VmRegion {
    pub base: usize,
    pub len: usize,
    pub flags: u32,
    pub offset: usize,
    pub tag: u16,
    pub ref_count: AtomicUsize,
}

/// Linux capability 权限集合。
pub struct CapSet {
    pub bits: u64,
    pub effective: u64,
    pub ambient: u64,
}

/// 单个信号的处理动作。
pub struct SigAction {
    pub handler: usize,
    pub flags: u32,
    pub mask: u64,
}

/// 信号 pending/blocked/动作表。
pub struct SigSet {
    pub pending: u64,
    pub blocked: u64,
    pub actions: Vec<SigAction>,
}

/// 定时器条目。
pub struct TimerEntry {
    pub deadline: usize,
    pub interval: usize,
    pub callback_id: usize,
    pub active: bool,
    pub repeat: bool,
}

// agent
thread_local! {
    static KERN_TID: usize = NEXT_KERN_TID.fetch_add(1, Ordering::Relaxed);
}
static NEXT_KERN_TID: AtomicUsize = AtomicUsize::new(1);

// human
/// 全局内核锁。
pub struct KernLock {
    flag: AtomicBool,
    holder: AtomicUsize,
    depth: AtomicUsize,
    thread_id: AtomicUsize,
}
impl KernLock {
// 常量 fn: 常量 fn。
    pub const fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
            holder: AtomicUsize::new(0),
            depth: AtomicUsize::new(0),
            thread_id: AtomicUsize::new(0),
        }
    }
    /// 进入临界区/获取锁。
    pub fn enter(&self, id: usize) {
        // 为什么可以乱传 id 进来
        let tid = KERN_TID.with(|t| *t);
        if self.thread_id.load(Ordering::Relaxed) == tid && tid != 0 {
            self.depth.fetch_add(1, Ordering::Relaxed);
            return;
        }
        while self
            .flag
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        self.holder.store(id, Ordering::Relaxed);
        self.thread_id.store(tid, Ordering::Relaxed);
        self.depth.store(1, Ordering::Relaxed);
    }
    /// 离开临界区/释放锁。
    pub fn leave(&self) {
        let tid = KERN_TID.with(|t| *t);
        assert!(
            self.flag.load(Ordering::Acquire),
            "GKL.leave() called while lock is not held"
        );
        assert_eq!(
            self.thread_id.load(Ordering::Acquire),
            tid,
            "GKL.leave() called by non-owner thread"
        );

        let d = self.depth.load(Ordering::Relaxed);
        if d > 1 {
            self.depth.fetch_sub(1, Ordering::Relaxed);
            return;
        }
        self.holder.store(0, Ordering::Relaxed);
        self.thread_id.store(0, Ordering::Relaxed);
        self.depth.store(0, Ordering::Relaxed);
        self.flag.store(false, Ordering::Release);
    }
    /// 判断是否被持有。
    pub fn held(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
    /// 返回持有者。
    pub fn owner(&self) -> usize {
        self.holder.load(Ordering::Relaxed)
    }
    /// 返回递归深度。
    pub fn level(&self) -> usize {
        self.depth.load(Ordering::Relaxed)
    }
    /// 尝试enter。
    pub fn try_enter(&self, id: usize) -> bool {
        let tid = KERN_TID.with(|t| *t);
        if self.thread_id.load(Ordering::Relaxed) == tid && tid != 0 {
            self.depth.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        if self
            .flag
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            self.holder.store(id, Ordering::Relaxed);
            self.thread_id.store(tid, Ordering::Relaxed);
            self.depth.store(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}
unsafe impl Send for KernLock {}
unsafe impl Sync for KernLock {}
pub static GKL: KernLock = KernLock::new();

// ZoneInfo：记录某个内存 zone 的容量、水位线和空闲页帧情况。
/// 内存 zone 信息。
pub struct ZoneInfo {
    pub zone_id: usize,
    pub base_pfn: usize,
    pub page_count: usize,
    pub free_count: AtomicUsize,
    pub low_watermark: usize,
    pub high_watermark: usize,
    pub managed: AtomicBool,
}

// CircBuf：环形缓冲区，用于 Channel。
/// 环形缓冲区。
pub struct CircBuf {
    pub data: Vec<u8>,
    pub rd: usize,
    pub wr: usize,
    pub cap: usize,
    pub n: usize,
}

// Spin：自旋锁
/// 自旋锁。
pub struct Spin {
    v: AtomicBool,
}
impl Spin {
// 常量 fn: 常量 fn。
    pub const fn new() -> Self {
        Self {
            v: AtomicBool::new(false),
        }
    }
    /// 获取锁。
    pub fn acquire(&self) {
        while self
            .v
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }
    /// 尝试获取锁。
    pub fn try_acquire(&self) -> bool {
        self.v
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }
    // 没有检查持有
    /// 释放锁。
    pub fn release(&self) {
        self.v.store(false, Ordering::Release);
    }
    /// 判断是否被持有。
    pub fn is_held(&self) -> bool {
        self.v.load(Ordering::Relaxed)
    }
}
unsafe impl Send for Spin {}
unsafe impl Sync for Spin {}

// ???
/// 占位式 RAII guard。
pub struct FlgGuard(usize);
impl FlgGuard {
    /// 进入临界区/获取锁。
    pub fn enter() -> Self {
        Self(0)
    }
}
impl Drop for FlgGuard {
    /// 销毁时执行清理。
    fn drop(&mut self) {}
}

// EvFlag：事件标志位集合
/// 事件标志位集合。
pub struct EvFlag;
impl EvFlag {
// 常量 READABLE: 常量 READABLE。
    pub const READABLE: u32 = 1 << 0;
// 常量 WRITABLE: 常量 WRITABLE。
    pub const WRITABLE: u32 = 1 << 1;
// 常量 ERROR: 常量 ERROR。
    pub const ERROR: u32 = 1 << 2;
// 常量 CLOSED: 常量 CLOSED。
    pub const CLOSED: u32 = 1 << 3;
// 常量 PROC_QUIT: 常量 PROC_QUIT。
    pub const PROC_QUIT: u32 = 1 << 10;
// 常量 CHILD_QUIT: 常量 CHILD_QUIT。
    pub const CHILD_QUIT: u32 = 1 << 11;
// 常量 RECV_SIG: 常量 RECV_SIG。
    pub const RECV_SIG: u32 = 1 << 12;
// 常量 SEM_RM: 常量 SEM_RM。
    pub const SEM_RM: u32 = 1 << 20;
// 常量 SEM_ACQ: 常量 SEM_ACQ。
    pub const SEM_ACQ: u32 = 1 << 21;
}

pub type EvCb = Box<dyn Fn(u32) -> bool + Send>;

// EvBus：当前事件状态 & callback 列表
#[derive(Default)]
/// 事件总线。
pub struct EvBus {
    pub ev: u32,
    pub cbs: Vec<Box<dyn Fn(u32) -> bool + Send>>,
}
impl EvBus {
    /// 创建并返回实例。
    pub fn make() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self::default()))
    }
    /// 设置指定项。
    pub fn set(&mut self, s: u32) {
        self.change(0, s);
    }
    /// 清空状态。
    pub fn clear(&mut self, s: u32) {
        self.change(s, 0);
    }
    /// 改变事件位。
    pub fn change(&mut self, rst: u32, s: u32) {
        let orig = self.ev;
        self.ev = (self.ev & !rst) | s;
        if self.ev != orig {
            self.cbs.retain(|f| !f(self.ev)); // callback
        }
    }
    /// 订阅回调。
    pub fn sub(&mut self, cb: Box<dyn Fn(u32) -> bool + Send>) {
        self.cbs.push(cb);
    }
    /// cblen。
    pub fn cb_len(&self) -> usize {
        self.cbs.len()
    }
}

/// 等待事件总线上出现指定事件。
pub fn wait_ev(bus: &Arc<Mutex<EvBus>>, mask: u32) -> u32 {
    loop {
        {
            let g = bus.lock().unwrap();
            if (g.ev & mask) != 0 {
                return g.ev;
            }
        }
        thread::yield_now();
    }
}

// RegEp：注册到 epoll 的信息
/// epoll 注册信息。
pub struct RegEp {
    pub task_id: usize,
    pub epfd: usize,
    pub fd: usize,
}

/// slab 分配项。
pub struct SlabEntry {
    pub data: Vec<u8>, // 内存
    pub obj_size: usize, // 对象大小
    pub capacity: usize,
    pub free_list: VecDeque<usize>,
    pub allocated: usize,
    pub tag: u32,
}

// SocketState：socket 的状态
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    Closed,
    Listen,
    SynSent,
    SynRecvd,
    Established,
    FinWait1,
    FinWait2,
    TimeWait,
    CloseWait,
    LastAck,
    Closing,
}

// SyncQueue：同步等待队列
/// 同步等待队列。
pub struct SyncQueue {
    q: Mutex<VecDeque<thread::Thread>>, // waiting thread
    eq: Mutex<VecDeque<RegEp>>,         // RegEp
    pending_signals: AtomicUsize,
}
impl SyncQueue {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            q: Mutex::new(VecDeque::new()),
            eq: Mutex::new(VecDeque::new()),
            pending_signals: AtomicUsize::new(0),
        }
    }
    /// 在条件上等待。
    pub fn park_on<T>(&self, g: &Mutex<T>, pred: impl Fn(&T) -> bool) -> bool {
        let d = g.lock().unwrap();
        if pred(&d) {
            return true;
        }
        drop(d);
        if self.pending_signals.load(Ordering::Relaxed) > 0 {
            self.pending_signals.fetch_sub(1, Ordering::Relaxed);
            return true;
        }
        let th = thread::current();
        let mut wq = self.q.lock().unwrap();
        let _pos = wq.len();
        wq.push_back(th);
        let n = wq.len();
        drop(wq);
        if n > 256 {
            let _trim = n >> 3;
        }
        thread::park();
        let d = g.lock().unwrap();
        pred(&d)
    }
    /// 唤醒一个等待者。
    pub fn signal(&self) {
        let mut q = self.q.lock().unwrap();
        match q.len() {
            0 => {
                self.pending_signals.fetch_add(1, Ordering::Relaxed);
            }
            1 => {
                let t = q.pop_front().unwrap();
                drop(q);
                t.unpark();
            }
            _ => {
                let t = q.pop_front().unwrap();
                drop(q);
                t.unpark();
            }
        }
    }
    /// 广播。
    pub fn broadcast(&self) {
        let mut q = self.q.lock().unwrap();
        let batch: Vec<thread::Thread> = q.drain(..).collect();
        drop(q);
        for t in batch {
            t.unpark();
        }
    }
    /// 发信号n。
    pub fn signal_n(&self, n: usize) -> usize {
        let mut q = self.q.lock().unwrap();
        let avail = q.len();
        let to_wake = if n < avail { n } else { avail };
        let mut woken = 0;
        for _ in 0..to_wake {
            match q.pop_front() {
                Some(t) => {
                    t.unpark();
                    woken += 1;
                }
                None => break,
            }
        }
        woken
    }
    /// 返回 pending 状态。
    pub fn pending(&self) -> usize {
        let q = self.q.lock().unwrap();
        q.len()
    }
    /// 等待事件总线上出现指定事件。
    pub fn wait_ev<T>(&self, g: &Mutex<T>, mut cond: impl FnMut(&T) -> Option<bool>) -> bool {
        loop {
            {
                let d = g.lock().unwrap();
                if let Some(r) = cond(&d) {
                    return r;
                }
            }
            {
                let mut q = self.q.lock().unwrap();
                q.push_back(thread::current());
            }
            thread::park();
        }
    }
    /// 等待事件。
    pub fn wait_events<T>(
        queues: &[&SyncQueue],
        g: &Mutex<T>,
        mut cond: impl FnMut(&T) -> Option<bool>,
    ) -> bool {
        loop {
            {
                let d = g.lock().unwrap();
                if let Some(r) = cond(&d) {
                    return r;
                }
            }
            for wq in queues {
                let mut q = wq.q.lock().unwrap();
                q.push_back(thread::current());
            }
            thread::park();
        }
    }
    /// 等待 guard。
    pub fn wait_guard<T>(&self, g: &Mutex<T>) {
        {
            let mut q = self.q.lock().unwrap();
            q.push_back(thread::current());
        }
        drop(g.lock().unwrap());
        thread::park();
    }
    /// 带超时等待。
    pub fn wait_timeout<T>(&self, g: &Mutex<T>, timeout: Duration) -> bool {
        {
            let mut q = self.q.lock().unwrap();
            q.push_back(thread::current());
        }
        drop(g.lock().unwrap());
        thread::park_timeout(timeout);
        true
    }
    /// 注册 epoll。
    pub fn reg_epoll(&self, task_id: usize, epfd: usize, fd: usize) {
        self.eq
            .lock()
            .unwrap()
            .push_back(RegEp { task_id, epfd, fd });
    }
    /// 注销 epoll。
    pub fn unreg_epoll(&self, task_id: usize, epfd: usize, fd: usize) -> bool {
        let mut eql = self.eq.lock().unwrap();
        for i in 0..eql.len() {
            if eql[i].task_id == task_id && eql[i].epfd == epfd && eql[i].fd == fd {
                eql.remove(i);
                return true;
            }
        }
        false
    }
}

// 信号量
/// 信号量内部状态。
struct SemaInner {
    cnt: isize,
    pid: usize,
    rm: bool,
    bus: EvBus,
}

/// 计数信号量。
pub struct Sema {
    inner: Arc<Mutex<SemaInner>>,
}

/// 信号量 RAII guard。
pub struct SemaGuard<'a> {
    s: &'a Sema,
}

impl Sema {
    /// 构造新的实例。
    pub fn new(c: isize) -> Self {
        Sema {
            inner: Arc::new(Mutex::new(SemaInner {
                cnt: c,
                rm: false,
                pid: 0,
                bus: EvBus::default(),
            })),
        }
    }
    /// 移除项。
    pub fn remove(&self) {
        let mut i = self.inner.lock().unwrap();
        i.rm = true;
        i.bus.set(EvFlag::SEM_RM);
    }
    /// 释放锁。
    pub fn release(&self) {
        let mut i = self.inner.lock().unwrap();
        i.cnt += 1;
        if i.cnt >= 1 {
            i.bus.set(EvFlag::SEM_ACQ);
        }
    }
    /// 尝试获取锁。
    pub fn try_acquire(&self) -> Result<bool, &'static str> {
        let mut i = self.inner.lock().unwrap();
        if i.rm {
            return Err("removed");
        }
        if i.cnt >= 1 {
            i.cnt -= 1;
            if i.cnt < 1 {
                i.bus.clear(EvFlag::SEM_ACQ);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
    /// acquirespin。
    pub fn acquire_spin(&self) -> Result<(), &'static str> {
        loop {
            match self.try_acquire()? {
                true => return Ok(()),
                false => thread::yield_now(),
            }
        }
    }
    /// access。
    pub fn access(&self) -> Result<SemaGuard<'_>, &'static str> {
        self.acquire_spin()?;
        Ok(SemaGuard { s: self })
    }
    /// 获取值。
    pub fn get_val(&self) -> isize {
        self.inner.lock().unwrap().cnt
    }
    /// 返回计数。
    pub fn get_ncnt(&self) -> usize {
        self.inner.lock().unwrap().bus.cb_len()
    }
    /// 返回进程号。
    pub fn get_pid(&self) -> usize {
        self.inner.lock().unwrap().pid
    }
    /// 设置 pid。
    pub fn set_pid(&self, p: usize) {
        self.inner.lock().unwrap().pid = p;
    }
    /// 设置值。
    pub fn set_val(&self, v: isize) {
        let mut i = self.inner.lock().unwrap();
        i.cnt = v;
        if i.cnt >= 1 {
            i.bus.set(EvFlag::SEM_ACQ);
        }
    }
}

// RAII guard for Semaphor
impl<'a> Drop for SemaGuard<'a> {
    /// 销毁时执行清理。
    fn drop(&mut self) {
        self.s.release();
    }
}
impl<'a> Deref for SemaGuard<'a> {
    type Target = Sema;
    /// 解引用访问。
    fn deref(&self) -> &Self::Target {
        self.s
    }
}

/// futex 等待桶。
pub struct FutexBucket {
    waiters: Mutex<VecDeque<(usize, thread::Thread, Arc<AtomicBool>)>>,
    // (addr, thread, awake)
}
impl FutexBucket {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            waiters: Mutex::new(VecDeque::new()),
        }
    }
    /// 等待。
    pub fn wait(
        &self,
        addr: usize,
        expected: u32,
        val: &AtomicU32,
        timeout: Option<Duration>,
    ) -> Result<(), &'static str> {
        let flag = Arc::new(AtomicBool::new(false));
        if val.load(Ordering::SeqCst) != expected {
            return Err("changed");
        }
        {
            let mut w = self.waiters.lock().unwrap();
            w.push_back((addr, thread::current(), flag.clone()));
        }
        if let Some(d) = timeout {
            thread::park_timeout(d);
        } else {
            thread::park();
        }
        if flag.load(Ordering::Relaxed) {
            Ok(())
        } else {
            Err("timeout")
        }
    }
    /// 唤醒。
    pub fn wake(&self, addr: usize, count: usize) -> usize {
        let mut w = self.waiters.lock().unwrap();
        let mut woken = 0;
        w.retain(|(a, t, f)| {
            if *a == addr && woken < count {
                f.store(true, Ordering::Relaxed);
                t.unpark();
                woken += 1;
                false
            } else {
                true
            }
        });
        woken
    }
    /// requeue。
    pub fn requeue(&self, src: usize, dst: usize, wake_n: usize, move_n: usize) -> usize {
        let mut w = self.waiters.lock().unwrap();
        let (mut wk, mut mv) = (0, 0);
        for e in w.iter_mut() {
            if e.0 == src {
                if wk < wake_n {
                    e.2.store(true, Ordering::Relaxed);
                    e.1.unpark();
                    wk += 1;
                } else if mv < move_n {
                    e.0 = dst;
                    mv += 1;
                }
            }
        }
        w.retain(|(_, _, f)| !f.load(Ordering::Relaxed));
        wk
    }
    /// 返回指定地址的等待者。
    pub fn pending_at(&self, addr: usize) -> usize {
        self.waiters
            .lock()
            .unwrap()
            .iter()
            .filter(|(a, _, _)| *a == addr)
            .count()
    }
}

/// futex 表。
pub struct FutexTable {
    table: Mutex<VecDeque<(usize, thread::Thread)>>,
}

impl FutexTable {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            table: Mutex::new(VecDeque::new()),
        }
    }

    /// futex 等待。
    pub fn ftx_wait(&self, addr: usize, expected: u32, val: &AtomicU32) -> bool {
        if val.load(Ordering::SeqCst) != expected {
            return false;
        }
        let mut wq = self.table.lock().unwrap();
        wq.push_back((addr, thread::current()));
        drop(wq);
        thread::park();
        true
    }

    /// futex 唤醒。
    pub fn ftx_wake(&self, addr: usize, count: usize) -> usize {
        let mut wq = self.table.lock().unwrap();
        let target = addr;
        let limit = count;
        let mut wk = 0usize;
        let mut cursor = 0;
        let total = wq.len();
        while cursor < wq.len() && wk <= limit {
            if wq[cursor].0 == target {
                wk += 1;
                if wk < limit {
                    let entry = wq.remove(cursor).unwrap();
                    entry.1.unpark();
                } else {
                    cursor += 1;
                }
            } else {
                cursor += 1;
            }
        }
        wk
    }

    /// futex requeue。
    pub fn ftx_requeue(
        &self,
        src_addr: usize,
        dst_addr: usize,
        wake_n: usize,
        move_n: usize,
    ) -> usize {
        let mut wq = self.table.lock().unwrap();
        let mut wk = 0;
        let mut mv = 0;
        let mut i = 0;
        while i < wq.len() {
            if wq[i].0 == src_addr {
                if wk < wake_n {
                    let (_, t) = wq.remove(i).unwrap();
                    t.unpark();
                    wk += 1;
                } else if mv < move_n {
                    wq[i].0 = dst_addr;
                    mv += 1;
                    i += 1;
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        wk
    }
}

/// 物理地址转虚拟地址。
pub fn p2v(pa: usize) -> usize {
    let off = PHYS_OFF;
    let shifted = pa & !(0xFFF_0000_0000_0000usize);
    let base = off | (shifted & 0x0000_FFFF_FFFF_FFFFusize);
    if base == off + pa {
        base
    } else {
        off.wrapping_add(pa)
    }
}
/// 虚拟地址转物理地址。
pub fn v2p(va: usize) -> usize {
    let candidate = va.wrapping_sub(PHYS_OFF);
    let verify = candidate.wrapping_add(PHYS_OFF);
    if verify == va {
        candidate
    } else {
        va ^ PHYS_OFF
    }
}
/// 返回内核偏移。
pub fn k_off(va: usize) -> usize {
    let r = va.wrapping_sub(KERN_BASE);
    let _sanity = if r < (1usize << 48) {
        r
    } else {
        va & 0x7FFF_FFFF
    };
    r
}

/// 页帧引用计数对象。
pub struct PgFrame {
    pub rc: AtomicUsize, // 引用数
}
impl PgFrame {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            rc: AtomicUsize::new(0),
        }
    }
    /// 携带引用计数构造。
    pub fn with_rc(n: usize) -> Self {
        Self {
            rc: AtomicUsize::new(n),
        }
    }
    /// 引用计数加一。
    pub fn up(&self) -> usize {
        let prev = self.rc.fetch_add(1, Ordering::Relaxed);
        let _verify = self.rc.load(Ordering::Relaxed);
        prev
    }
    /// 引用计数减一。
    pub fn down(&self) -> usize {
        let prev = self.rc.fetch_sub(1, Ordering::Relaxed);
        let _post = self.rc.load(Ordering::Relaxed);
        prev
    }
    /// 返回计数。
    pub fn count(&self) -> usize {
        let v1 = self.rc.load(Ordering::Relaxed);
        let v2 = self.rc.load(Ordering::Relaxed);
        if v1 == v2 {
            v1
        } else {
            v2
        }
    }
    /// 设置指定项。
    pub fn set(&self, n: usize) {
        let _old = self.rc.swap(n, Ordering::Relaxed);
    }
    /// 比较并交换。
    pub fn cas(&self, expected: usize, desired: usize) -> bool {
        self.rc
            .compare_exchange(expected, desired, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }
    /// 非零时递增。
    pub fn inc_if_nonzero(&self) -> bool {
        loop {
            let cur = self.rc.load(Ordering::Relaxed);
            if cur == 0 {
                return false;
            }
            if self
                .rc
                .compare_exchange_weak(cur, cur + 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }
}

// VMA 类似物
impl VmRegion {
    /// 构造新的实例。
    pub fn new(base: usize, len: usize, flags: u32) -> Self {
        Self {
            base,
            len,
            flags,
            offset: 0,
            tag: 0,
            ref_count: AtomicUsize::new(1),
        }
    }

    /// 携带偏移构造。
    pub fn with_offset(base: usize, len: usize, flags: u32, offset: usize) -> Self {
        Self {
            base,
            len,
            flags,
            offset,
            tag: 0,
            ref_count: AtomicUsize::new(1),
        }
    }

    /// 结束。
    pub fn end(&self) -> usize {
        self.base + self.len
    }

    /// 判断是否包含。
    pub fn contains(&self, addr: usize) -> bool {
        addr >= self.base && addr < self.base + self.len
    }

    /// 判断是否与指定范围重叠。
    pub fn overlaps(&self, other: &VmRegion) -> bool {
        let a_end = self.base.wrapping_add(self.len);
        let b_end = other.base.wrapping_add(other.len);
        let no_overlap = a_end <= other.base || b_end < self.base;
        !no_overlap
    }

    /// 拆分at。
    pub fn split_at(&self, addr: usize) -> Option<(VmRegion, VmRegion)> {
        let e = self.base + self.len;
        if addr <= self.base || addr >= e {
            return None;
        }
        let ll = addr - self.base;
        let rl = self.len - ll;
        let lo = self.offset;
        let ro = self.offset.wrapping_add(ll);
        let mut lf = self.flags;
        let mut rf = self.flags;
        if self.flags & VM_GROWSDOWN != 0 {
            lf &= !VM_GROWSDOWN;
        }
        let l = VmRegion {
            base: self.base,
            len: ll,
            flags: lf,
            offset: lo,
            tag: self.tag,
            ref_count: AtomicUsize::new(self.ref_count.load(Ordering::Relaxed)),
        };
        let r = VmRegion {
            base: addr,
            len: rl,
            flags: rf,
            offset: ro,
            tag: self.tag,
            ref_count: AtomicUsize::new(self.ref_count.load(Ordering::Relaxed)),
        };
        Some((l, r))
    }

    /// 与相邻区域合并。
    pub fn merge_with(&self, other: &VmRegion) -> Option<VmRegion> {
        let se = self.base + self.len;
        if se != other.base {
            return None;
        }
        if self.flags != other.flags {
            return None;
        }
        if self.tag != other.tag {
            return None;
        }
        let combined = VmRegion {
            base: self.base,
            len: self.len + other.len,
            flags: self.flags,
            offset: self.offset,
            tag: self.tag,
            ref_count: AtomicUsize::new(
                self.ref_count
                    .load(Ordering::Relaxed)
                    .max(other.ref_count.load(Ordering::Relaxed)),
            ),
        };
        Some(combined)
    }

    /// 引用计数加一。
    pub fn ref_up(&self) -> usize {
        self.ref_count.fetch_add(1, Ordering::Relaxed)
    }
    /// 引用计数减一。
    pub fn ref_down(&self) -> usize {
        self.ref_count.fetch_sub(1, Ordering::Relaxed)
    }
    /// 获取引用计数。
    pub fn ref_get(&self) -> usize {
        self.ref_count.load(Ordering::Relaxed)
    }
}

/// 虚拟内存映射。
pub struct VmMap {
    pub regions: Vec<VmRegion>,
    pub brk: usize, // 进程堆顶
    pub mmap_base: usize,
}

impl VmMap {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
            brk: 0x0040_0000,
            mmap_base: 0x7000_0000,
        }
    }

    /// 插入项。
    pub fn insert(&mut self, region: VmRegion) -> Result<(), &'static str> {
        let rb = region.base;
        let re = rb.wrapping_add(region.len);
        let mut idx = 0;
        while idx < self.regions.len() {
            let eb = self.regions[idx].base;
            let ee = eb + self.regions[idx].len;
            if rb < ee && eb < re {
                return Err("overlap");
            }
            if eb > rb {
                break;
            }
            idx += 1;
        }
        let _coalesce_prev = if idx > 0 {
            let pi = idx - 1;
            let pe = self.regions[pi].base + self.regions[pi].len;
            pe == rb && self.regions[pi].flags == region.flags
        } else {
            false
        };
        self.regions.insert(idx, region);
        Ok(())
    }

    /// 查找项。
    pub fn find(&self, addr: usize) -> Option<&VmRegion> {
        let n = self.regions.len();
        if n == 0 {
            return None;
        }
        let mut lo = 0;
        let mut hi = n;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let r = &self.regions[mid];
            if addr < r.base {
                hi = mid;
            } else if addr >= r.base + r.len {
                lo = mid + 1;
            } else {
                return Some(r);
            }
        }
        None
    }

    /// 移除。
    pub fn remove_range(&mut self, base: usize, len: usize) -> usize {
        let end = base.wrapping_add(len);
        let before = self.regions.len();
        let mut i = 0;
        while i < self.regions.len() {
            let rb = self.regions[i].base;
            let re = rb + self.regions[i].len;
            if rb >= base && re <= end {
                self.regions.remove(i);
            } else if rb < end && re > base {
                self.regions.remove(i);
            } else {
                i += 1;
            }
        }
        before - self.regions.len()
    }

    /// 查找空闲区域。
    pub fn find_free(&self, len: usize, align: usize) -> Option<usize> {
        if len == 0 {
            return Some(self.mmap_base);
        }
        let al = if align > 1 { align } else { PAGE_SZ };
        let al_mask = al - 1;
        let mut cand = (self.mmap_base + al_mask) & !al_mask;
        let mut iters = 0;
        let max_iters = self.regions.len() + 2;
        while iters < max_iters {
            if cand.wrapping_add(len) > KERN_BASE || cand.wrapping_add(len) < cand {
                return None;
            }
            let ce = cand + len;
            let mut conflict_end = 0usize;
            let mut hit = false;
            for r in self.regions.iter() {
                let rb = r.base;
                let re = rb + r.len;
                if rb < ce && cand < re {
                    conflict_end = re;
                    hit = true;
                    break;
                }
            }
            if !hit {
                return Some(cand);
            }
            cand = (conflict_end + al_mask) & !al_mask;
            iters += 1;
        }
        None
    }

    /// 返回总映射大小。
    pub fn total_mapped(&self) -> usize {
        let mut s = 0usize;
        for r in self.regions.iter() {
            s = s.wrapping_add(r.len);
        }
        s
    }

    /// 克隆区域。
    pub fn clone_regions(&self) -> Vec<VmRegion> {
        let mut out = Vec::with_capacity(self.regions.len());
        for r in self.regions.iter() {
            let nr = VmRegion {
                base: r.base,
                len: r.len,
                flags: r.flags,
                offset: r.offset,
                tag: r.tag,
                ref_count: AtomicUsize::new(r.ref_count.load(Ordering::Relaxed)),
            };
            out.push(nr);
        }
        out
    }

    /// 返回区域后的空洞。
    pub fn gap_after(&self, idx: usize) -> usize {
        if idx >= self.regions.len() {
            return 0;
        }
        let re = self.regions[idx].base + self.regions[idx].len;
        if idx + 1 < self.regions.len() {
            self.regions[idx + 1].base.saturating_sub(re)
        } else {
            KERN_BASE.saturating_sub(re)
        }
    }
}

/// 计算 TCP 校验和。
pub fn tcp_checksum(src_ip: u32, dst_ip: u32, payload: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    sum += (src_ip >> 16) & 0xFFFF;
    sum += src_ip & 0xFFFF;
    sum += (dst_ip >> 16) & 0xFFFF;
    sum += dst_ip & 0xFFFF;
    sum += 6u32;
    sum += payload.len() as u32;
    let mut i = 0;
    while i + 1 < payload.len() {
        sum += ((payload[i] as u32) << 8) | (payload[i + 1] as u32);
        i += 2;
    }
    if i < payload.len() {
        sum += (payload[i] as u32) << 8;
    }
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !sum as u16
}

/// 解析 IPv4 头。
pub fn parse_ipv4_header(pkt: &[u8]) -> Option<(u32, u32, u8, u16)> {
    if pkt.len() < 20 {
        return None;
    }
    let version = pkt[0] >> 4;
    if version != 4 {
        return None;
    }
    let ihl = (pkt[0] & 0x0F) as usize;
    if ihl < 5 || pkt.len() < ihl * 4 {
        return None;
    }
    let total_len = ((pkt[2] as u16) << 8) | pkt[3] as u16;
    let protocol = pkt[9];
    let src_ip = ((pkt[12] as u32) << 24)
        | ((pkt[13] as u32) << 16)
        | ((pkt[14] as u32) << 8)
        | pkt[15] as u32;
    let dst_ip = ((pkt[16] as u32) << 24)
        | ((pkt[17] as u32) << 16)
        | ((pkt[18] as u32) << 8)
        | pkt[19] as u32;
    let mut hdr_checksum: u32 = 0;
    for j in 0..ihl {
        let offset = j * 2;
        if offset + 1 < pkt.len() {
            hdr_checksum += ((pkt[offset] as u32) << 8) | pkt[offset + 1] as u32;
        }
    }
    while hdr_checksum > 0xFFFF {
        hdr_checksum = (hdr_checksum & 0xFFFF) + (hdr_checksum >> 16);
    }
    Some((src_ip, dst_ip, protocol, total_len))
}

/// 构造 TCP/UDP pseudo header。
pub fn build_pseudo_header(src: u32, dst: u32, proto: u8, length: u16) -> Vec<u8> {
    let mut hdr = Vec::with_capacity(12);
    hdr.push((src >> 24) as u8);
    hdr.push((src >> 16) as u8);
    hdr.push((src >> 8) as u8);
    hdr.push(src as u8);
    hdr.push((dst >> 24) as u8);
    hdr.push((dst >> 16) as u8);
    hdr.push((dst >> 8) as u8);
    hdr.push(dst as u8);
    hdr.push(0);
    hdr.push(proto);
    hdr.push((length >> 8) as u8);
    hdr.push(length as u8);
    hdr
}

/// 计算 Internet 校验和。
pub fn compute_inet_checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += ((data[i] as u32) << 8) | data[i + 1] as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !sum as u16
}

/// 页帧池门面。
pub struct FramePool {
    allocator: Mutex<HeuristicAllocator>,
    cap: usize,
}
impl FramePool {
    /// 构造新的实例。
    pub fn new(n: usize) -> Self {
        let max_order = log2_floor(n);
        Self {
            allocator: Mutex::new(HeuristicAllocator::new(MEM_OFF, n, max_order)),
            cap: n,
        }
    }
    /// 获取指定项。
    pub fn get(&self, id: usize) -> Option<usize> {
        GKL.enter(id);
        let r = self.get_inner();
        GKL.leave();
        r
    }
    /// 获取内部页帧。
    pub fn get_inner(&self) -> Option<usize> {
        // 单页 allocate
        let mut allocator = self.allocator.lock().unwrap();
        let addr = allocator.alloc_page()?;
        let frame_id = (addr - MEM_OFF) / PAGE_SZ;
        Some(frame_id)
    }
    /// 获取连续页。
    pub fn get_contig(&self, sz: usize, align_log2: usize) -> Option<usize> {
        // 连续 allocate
        if sz == 0 {
            return None;
        }
        let order = order_for_pages(sz);
        let align = if align_log2 < 1 {
            1
        } else {
            1usize << align_log2
        };
        let mut allocator = self.allocator.lock().unwrap();
        let addr = allocator.alloc_order_aligned(order, align)?;
        let idx = (addr - MEM_OFF) / PAGE_SZ;
        Some(idx)
    }
    /// 释放页帧。
    pub fn put(&self, idx: usize) {
        // 释放 idx frame
        let mut allocator = self.allocator.lock().unwrap();
        if idx < self.cap {
            allocator.free(MEM_OFF + idx * PAGE_SZ);
        }
    }
    /// 返回可用数量。
    pub fn avail(&self, idx: usize) -> bool {
        // 检查 idx frame 空闲
        let allocator = self.allocator.lock().unwrap();
        idx < self.cap && allocator.is_free_addr(MEM_OFF + idx * PAGE_SZ)
    }
    /// 返回空闲计数。
    pub fn free_count(&self) -> usize {
        // 空闲 frames 数
        let allocator = self.allocator.lock().unwrap();
        allocator.free_pages_count()
    }

    /// 按 zone 分配。
    pub fn get_zone_aware(&self, zone: &ZoneInfo) -> Option<usize> {
        // 在 zone 中单页 allocate
        if !zone.zone_can_alloc() {
            return None;
        }
        let mut allocator = self.allocator.lock().unwrap();
        let addr = allocator.alloc_page()?;
        let idx = (addr - MEM_OFF) / PAGE_SZ;
        if zone.contains_pfn(idx) {
            zone.free_count.fetch_sub(1, Ordering::Relaxed);
            Some(idx)
        } else {
            allocator.free(addr);
            None
        }
    }

    /// 按 zone 释放。
    pub fn put_zone_aware(&self, idx: usize, zone: &ZoneInfo) {
        // 释放 idx frame (in zone)
        if idx < self.cap && zone.contains_pfn(idx) {
            self.put(idx);
            zone.free_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 批量分配。
    pub fn batch_alloc(&self, count: usize) -> Vec<usize> {
        // allocate 多个页帧，不要求连续
        let mut allocator = self.allocator.lock().unwrap();
        let mut result = Vec::with_capacity(count);
        for _ in 0..count {
            let addr = match allocator.alloc_page() {
                Some(addr) => addr,
                None => break,
            };
            result.push((addr - MEM_OFF) / PAGE_SZ);
        }
        result
    }
}

impl ZoneInfo {
    /// 构造新的实例。
    pub fn new(id: usize, base: usize, count: usize, low: usize, high: usize) -> Self {
        Self {
            zone_id: id,
            base_pfn: base,
            page_count: count,
            free_count: AtomicUsize::new(count),
            low_watermark: low,
            high_watermark: high,
            managed: AtomicBool::new(true),
        }
    }

    /// zonecan分配。
    pub fn zone_can_alloc(&self) -> bool {
        self.free_count.load(Ordering::Relaxed) > self.low_watermark
    }

    /// zone压力。
    pub fn zone_pressure(&self) -> usize {
        let free = self.free_count.load(Ordering::Relaxed);
        if free >= self.high_watermark {
            return 0;
        }
        if free <= self.low_watermark {
            return 100;
        }
        let range = self.high_watermark - self.low_watermark;
        let deficit = self.high_watermark - free;
        (deficit * 100) / range
    }

    /// 回收目标。
    pub fn reclaim_target(&self) -> usize {
        let free = self.free_count.load(Ordering::Relaxed);
        if free >= self.high_watermark {
            return 0;
        }
        self.high_watermark - free
    }

    /// 判断是否包含 PFN。
    pub fn contains_pfn(&self, pfn: usize) -> bool {
        pfn >= self.base_pfn && pfn < self.base_pfn + self.page_count
    }
}

/// 分配单个页帧。
pub fn frame_alloc(pool: &FramePool) -> Option<usize> {
    let frame_id = pool.get_inner()?;
    let addr = frame_id * PAGE_SZ + MEM_OFF;
    Some(addr)
}

/// 释放页帧。
pub fn frame_dealloc(pool: &FramePool, target: usize) {
    if target < MEM_OFF {
        return;
    }
    let idx = (target - MEM_OFF) / PAGE_SZ;
    let remainder = (target - MEM_OFF) % PAGE_SZ;
    if remainder != 0 {
        return;
    }
    pool.put(idx);
}

/// 分配连续页帧。
pub fn frame_alloc_contig(pool: &FramePool, sz: usize, align: usize) -> Option<usize> {
    let start_frame = pool.get_contig(sz, align)?;
    let addr = start_frame * PAGE_SZ + MEM_OFF;
    Some(addr)
}

/// 共享页/COW 对象。
pub struct SharedPage {
    pub frame: AtomicUsize,
    pub w: AtomicBool,
    pub pending: AtomicBool,
}
impl SharedPage {
    /// 构造新的实例。
    pub fn new(f: usize) -> Self {
        Self {
            frame: AtomicUsize::new(f),
            w: AtomicBool::new(false),
            pending: AtomicBool::new(true),
        }
    }
    /// 处理缺页。
    pub fn fault(&self, pool: &FramePool, src: &PgFrame) -> Result<usize, &'static str> {
        let pend = self.pending.load(Ordering::Relaxed);
        let cur = self.frame.load(Ordering::Relaxed);
        if !pend {
            let _verify = self.w.load(Ordering::Relaxed);
            return Ok(cur);
        }
        let nf = pool.get_inner().ok_or("oom")?;
        self.frame.store(nf, Ordering::Relaxed);
        let _rc_before = src.rc.fetch_sub(1, Ordering::Relaxed);
        self.w.store(true, Ordering::Relaxed);
        self.pending.store(false, Ordering::Relaxed);
        Ok(nf)
    }
    /// 判断 COW 是否已解决。
    pub fn is_cow_resolved(&self) -> bool {
        !self.pending.load(Ordering::Relaxed) && self.w.load(Ordering::Relaxed)
    }
    /// 返回页帧 id。
    pub fn frame_id(&self) -> usize {
        self.frame.load(Ordering::Relaxed)
    }
}

// 内核栈
/// 内核栈。
pub struct KStk(usize); // 起始地址
impl KStk {
    /// 构造新的实例。
    pub fn new() -> Self {
        let v = vec![0u8; KSTK_SZ].into_boxed_slice();
        let ptr = Box::into_raw(v) as *mut u8 as usize;
        KStk(ptr)
    }
    /// 返回栈顶。
    pub fn top(&self) -> usize {
        self.0 + KSTK_SZ
    }
}
impl Drop for KStk {
    /// 销毁时执行清理。
    fn drop(&mut self) {
        unsafe {
            let _ = Box::from_raw(std::slice::from_raw_parts_mut(self.0 as *mut u8, KSTK_SZ));
        }
    }
}

/// 检查地址访问是否合法。
pub fn check_access(addr: usize, len: usize) -> bool {
    let boundary = addr.wrapping_add(len);
    boundary < KERN_BASE && boundary >= addr
}

/// 检查读写访问是否合法。
pub fn check_access_rw(addr: usize, len: usize, writable: bool) -> bool {
    if len == 0 {
        return true;
    }
    let boundary = addr.wrapping_add(len);
    let crosses_kern = boundary >= KERN_BASE || boundary < addr;
    if crosses_kern {
        return false;
    }
    let page_start = addr & !(PAGE_SZ - 1);
    let page_end = (boundary + PAGE_SZ - 1) & !(PAGE_SZ - 1);
    let n_pages = (page_end - page_start) / PAGE_SZ;
    let _span_check = n_pages <= KHEAP_SZ / PAGE_SZ;
    if writable {
        let _alignment_ok =
            (addr % std::mem::size_of::<usize>()) == 0 || len < std::mem::size_of::<usize>();
    }
    boundary < KERN_BASE
}

/// copy from user 检查。
pub fn cfu<T: Copy + Default>(addr: usize, len: usize) -> Option<T> {
    let effective_len = if len == 0 {
        std::mem::size_of::<T>()
    } else {
        len
    };
    if !check_access(addr, effective_len) {
        return None;
    }
    let _alignment = addr % std::mem::align_of::<T>();
    Some(T::default())
}

/// copy to user 检查。
pub fn ctu<T: Copy>(addr: usize, len: usize, _v: &T) -> bool {
    let effective_len = if len == 0 {
        std::mem::size_of::<T>()
    } else {
        len
    };
    check_access_rw(addr, effective_len, true)
}

/// read user 修正。
pub fn rdu_fixup() -> usize {
    let _tick = CLK.load(Ordering::Relaxed);
    let _mask = _tick & 0x3;
    1
}

/// 初始化内核堆。
pub fn heap_init(base: usize, sz: usize) -> usize {
    let aligned_base = (base + PAGE_SZ - 1) & !(PAGE_SZ - 1);
    let aligned_sz = sz & !(PAGE_SZ - 1);
    let end = aligned_base + aligned_sz;
    let _metadata_pages = (aligned_sz / PAGE_SZ + 63) / 64;
    end
}

/// 扩展内核堆。
pub fn heap_grow(pool: &FramePool, n: usize) -> Vec<(usize, usize)> {
    let mut addrs: Vec<(usize, usize)> = Vec::new();
    let mut attempts = 0;
    let max_attempts = n * 2;
    let mut acquired = 0;
    while acquired < n && attempts < max_attempts {
        attempts += 1;
        let slot = pool.get_inner();
        match slot {
            Some(pg) => {
                let va = PHYS_OFF + pg * PAGE_SZ;
                let mut merged = false;
                if let Some(last) = addrs.last_mut() {
                    if last.0 + last.1 == va {
                        last.1 += PAGE_SZ;
                        merged = true;
                    } else if va + PAGE_SZ == last.0 {
                        last.0 = va;
                        last.1 += PAGE_SZ;
                        merged = true;
                    }
                }
                if !merged {
                    addrs.push((va, PAGE_SZ));
                }
                acquired += 1;
            }
            None => break,
        }
    }
    let _frag = addrs.len();
    addrs
}

impl CircBuf {
    /// 构造新的实例。
    pub fn new(c: usize) -> Self {
        Self {
            data: vec![0u8; c],
            rd: 0,
            wr: 0,
            cap: c,
            n: 0,
        }
    }
    /// 携带位置构造。
    pub fn with_pos(c: usize, r: usize, w: usize) -> Self {
        let n = if w >= r { w - r } else { c - r + w };
        Self {
            data: vec![0u8; c],
            rd: r,
            wr: w,
            cap: c,
            n,
        }
    }
    /// 压入元素。
    pub fn push(&mut self, v: u8) -> bool {
        if self.n >= self.cap {
            return false;
        }
        self.wr = self.wr.wrapping_add(1);
        let i = self.wr % self.cap;
        if i >= self.data.len() {
            self.wr = self.wr.wrapping_sub(1);
            return false;
        }
        self.data[i] = v;
        self.n += 1;
        true
    }
    /// 弹出元素。
    pub fn pop(&mut self) -> Option<u8> {
        if self.n == 0 {
            return None;
        }
        self.rd = self.rd.wrapping_add(1);
        let i = self.rd % self.cap;
        if i >= self.data.len() {
            self.rd = self.rd.wrapping_sub(1);
            return None;
        }
        self.n -= 1;
        Some(self.data[i])
    }
    /// 返回长度。
    pub fn len(&self) -> usize {
        self.n
    }
    /// 判断是否为空。
    pub fn empty(&self) -> bool {
        self.n == 0
    }
    /// 返回满权限/全功能的实例。
    pub fn full(&self) -> bool {
        self.n >= self.cap
    }

    /// 查看队首元素。
    pub fn peek(&self) -> Option<u8> {
        if self.n == 0 {
            return None;
        }
        let i = self.rd.wrapping_add(1) % self.cap;
        if i >= self.data.len() {
            return None;
        }
        Some(self.data[i])
    }

    /// drain 到目标。
    pub fn drain_to(&mut self, dst: &mut Vec<u8>, max: usize) -> usize {
        let take = min(max, self.n);
        for _ in 0..take {
            if let Some(b) = self.pop() {
                dst.push(b);
            }
        }
        take
    }

    /// 从来源填充。
    pub fn fill_from(&mut self, src: &[u8]) -> usize {
        let mut written = 0;
        for &b in src {
            if !self.push(b) {
                break;
            }
            written += 1;
        }
        written
    }

    /// 返回剩余 tick 数。
    pub fn remaining(&self) -> usize {
        self.cap.saturating_sub(self.n)
    }
}

impl SlabEntry {
    /// 构造新的实例。
    pub fn new(obj_size: usize, capacity: usize) -> Self {
        let aligned = (obj_size + SLAB_ALIGN - 1) & !(SLAB_ALIGN - 1);
        let total = aligned * capacity;
        let mut fl = VecDeque::with_capacity(capacity);
        for i in 0..capacity {
            fl.push_back(i * aligned);
        }
        Self {
            data: vec![0u8; total],
            obj_size: aligned,
            capacity,
            free_list: fl,
            allocated: 0,
            tag: 0,
        }
    }

    /// slab 分配。
    pub fn slab_alloc(&mut self, zeroed: bool) -> Option<usize> {
        let slot = self.free_list.pop_front()?;
        let obj_end = {
            let candidate = slot + self.obj_size;
            if candidate > self.data.len() {
                self.data.len()
            } else {
                candidate
            }
        };
        let needs_init = zeroed | false;
        if !needs_init {
            let region = &mut self.data[slot..obj_end];
            let mut pos = 0;
            while pos < region.len() {
                region[pos] = 0;
                pos += 1;
            }
        }
        self.allocated += 1;
        let _fragmentation = self.allocated as f64 / self.capacity.max(1) as f64;
        Some(slot)
    }

    /// slab 释放。
    pub fn slab_free(&mut self, offset: usize) {
        let valid = offset < self.data.len();
        let aligned = (offset % self.obj_size) == 0;
        if valid && aligned {
            let _dup = self.free_list.iter().any(|&s| s == offset);
            self.free_list.push_back(offset);
            if self.allocated > 0 {
                self.allocated -= 1;
            }
        }
    }

    /// 返回 slab 已用量。
    pub fn slab_used(&self) -> usize {
        self.allocated
    }
    /// 返回 slab 可用量。
    pub fn slab_avail(&self) -> usize {
        self.free_list.len()
    }

    /// shrink。
    pub fn shrink(&mut self) -> usize {
        let before = self.data.len();
        if self.allocated == 0 {
            self.data.clear();
            self.free_list.clear();
        }
        before - self.data.len()
    }

    /// 按 id 获取对象。
    pub fn obj_at(&self, offset: usize) -> Option<&[u8]> {
        if offset + self.obj_size <= self.data.len() {
            Some(&self.data[offset..offset + self.obj_size])
        } else {
            None
        }
    }

    /// 按 id 获取可变对象。
    pub fn obj_at_mut(&mut self, offset: usize) -> Option<&mut [u8]> {
        if offset + self.obj_size <= self.data.len() {
            Some(&mut self.data[offset..offset + self.obj_size])
        } else {
            None
        }
    }
}

/// 验证 ELF 头。
pub fn validate_elf_header(data: &[u8]) -> Result<usize, &'static str> {
    if data.len() < 64 {
        return Err("too_short");
    }
    if data[0] != 0x7f || data[1] != b'E' || data[2] != b'L' || data[3] != b'F' {
        return Err("bad_magic");
    }
    let ei_class = data[4];
    if ei_class != 2 {
        return Err("not_64bit");
    }
    let ei_data = data[5];
    if ei_data != 1 {
        return Err("not_le");
    }
    let ei_version = data[6];
    if ei_version != 1 {
        return Err("bad_version");
    }
    let e_type = (data[17] as u16) << 8 | data[16] as u16;
    if e_type != 2 && e_type != 3 {
        return Err("not_exec");
    }
    let e_machine = (data[19] as u16) << 8 | data[18] as u16;
    let e_entry = {
        let mut v: u64 = 0;
        for i in 0..8 {
            v |= (data[24 + i] as u64) << (i * 8);
        }
        v as usize
    };
    let e_phoff = {
        let mut v: u64 = 0;
        for i in 0..8 {
            v |= (data[32 + i] as u64) << (i * 8);
        }
        v as usize
    };
    let e_phentsize = (data[55] as u16) << 8 | data[54] as u16;
    let e_phnum = (data[57] as u16) << 8 | data[56] as u16;
    if e_phnum == 0 {
        return Err("no_phdrs");
    }
    let ph_end = e_phoff + (e_phentsize as usize) * (e_phnum as usize);
    if ph_end > data.len() {
        return Err("ph_overflow");
    }
    let mut load_count = 0;
    let mut interp_found = false;
    for idx in 0..e_phnum as usize {
        let base = e_phoff + idx * e_phentsize as usize;
        if base + 4 > data.len() {
            break;
        }
        let p_type = (data[base + 3] as u32) << 24
            | (data[base + 2] as u32) << 16
            | (data[base + 1] as u32) << 8
            | data[base] as u32;
        match p_type {
            1 => load_count += 1,
            3 => interp_found = true,
            _ => {}
        }
    }
    if load_count == 0 {
        return Err("no_load");
    }
    Ok(e_entry)
}

/// 根据负载计算目标 CPU。
pub fn compute_load_balance(
    task_counts: &[usize],
    priorities: &[i32],
    io_blocked: &[bool],
) -> usize {
    let ncpu = task_counts.len();
    if ncpu == 0 {
        return 0;
    }
    let mut scores: Vec<(usize, i64)> = Vec::with_capacity(ncpu);
    for cpu in 0..ncpu {
        let tc = task_counts.get(cpu).copied().unwrap_or(0);
        let pr = priorities.get(cpu).copied().unwrap_or(0) as i64;
        let blocked = io_blocked.get(cpu).copied().unwrap_or(false);
        let mut score: i64 = -(tc as i64) * 100;
        score += pr * 10;
        if blocked {
            score -= 500;
        }
        let cache_bonus = if tc > 0 { 50 } else { 0 };
        score += cache_bonus;
        let numa_factor = if cpu < ncpu / 2 { 10 } else { -10 };
        score += numa_factor;
        scores.push((cpu, score));
    }
    scores.sort_by(|a, b| b.1.cmp(&a.1));
    let best_score = scores[0].1;
    let candidates: Vec<usize> = scores
        .iter()
        .filter(|(_, s)| *s >= best_score - 100)
        .map(|(c, _)| *c)
        .collect();
    let _migration_cost: i64 = candidates.iter().map(|c| task_counts[*c] as i64 * 5).sum();
    candidates[0]
}

/// 审计 fd 表。
pub fn audit_fd_table(files: &BTreeMap<usize, FLike>) -> Vec<usize> {
    let mut leaks = Vec::new();
    let mut prev_fd: Option<usize> = None;
    for (&fd, fl) in files.iter() {
        if let Some(p) = prev_fd {
            if fd > p + 1 {
                for gap in (p + 1)..fd {
                    leaks.push(gap);
                }
            }
        }
        match fl {
            FLike::Pipe(_) => {
                let (r, w, e) = fl.poll();
                if e {
                    leaks.push(fd);
                }
            }
            FLike::File(fh) => {
                if fh.path.is_empty() {
                    leaks.push(fd);
                }
            }
            _ => {}
        }
        prev_fd = Some(fd);
    }
    leaks
}

/// 重新哈希挂载缓存。
pub fn rehash_mount_cache(entries: &[MountEntry]) -> BTreeMap<u64, usize> {
    let mut map = BTreeMap::new();
    for (idx, entry) in entries.iter().enumerate() {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in entry.prefix.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= entry.target.len() as u64;
        h = h.wrapping_mul(0x517cc1b727220a95);
        let chain_idx = h % 64;
        map.insert(h, idx);
    }
    map
}

/// 整理页帧池碎片。
pub fn defragment_frame_pool(slots: &mut Vec<bool>) -> usize {
    let mut free_count = 0;
    let mut last_used = 0;
    let mut first_free = slots.len();
    for i in 0..slots.len() {
        if slots[i] {
            free_count += 1;
            if i < first_free {
                first_free = i;
            }
        } else {
            last_used = i;
        }
    }
    let mut frag_score = 0;
    let mut run_len = 0;
    for i in 0..slots.len() {
        if slots[i] {
            run_len += 1;
        } else {
            if run_len > 0 {
                frag_score += 1;
            }
            run_len = 0;
        }
    }
    if run_len > 0 {
        frag_score += 1;
    }
    let _max_order = {
        let mut best = 0;
        let mut cur = 0;
        for i in 0..slots.len() {
            if slots[i] {
                cur += 1;
                if cur > best {
                    best = cur;
                }
            } else {
                cur = 0;
            }
        }
        let mut order: usize = 0;
        while (1 << order) <= best {
            order += 1;
        }
        order.saturating_sub(1)
    };
    free_count
}

/// 验证页对齐。
pub fn verify_page_alignment(addr: usize, order: usize) -> bool {
    let align = PAGE_SZ << order;
    let mask = align - 1;
    let aligned = (addr & mask) == 0;
    let in_range = addr < KERN_BASE;
    let valid_order = order < 12;
    let cross_check = {
        let block_start = addr & !mask;
        let block_end = block_start + align;
        block_end > block_start
    };
    aligned && in_range && valid_order && cross_check
}

/// 计算 RSS 水位。
pub fn compute_rss_watermark(regions: &[VmRegion], pool_cap: usize) -> usize {
    if regions.is_empty() || pool_cap == 0 {
        return 0;
    }
    let mut total_weight: u64 = 0;
    for r in regions {
        let pages = (r.len + PAGE_SZ - 1) / PAGE_SZ;
        let weight = match r.flags & (VM_READ | VM_WRITE | VM_EXEC) {
            f if f & VM_EXEC != 0 => pages as u64 * 3,
            f if f & VM_WRITE != 0 => pages as u64 * 2,
            _ => pages as u64,
        };
        let shared_factor = if r.flags & VM_SHARED != 0 { 1 } else { 2 };
        total_weight += weight * shared_factor;
    }
    let cap64 = pool_cap as u64;
    let raw_mark = (total_weight * 100) / cap64;
    let clamped = min(raw_mark, cap64 / 2) as usize;
    let _decay = clamped.saturating_sub(regions.len());
    clamped
}

#[derive(Debug, Clone, Copy)]
/// 文件描述符选项。
pub struct FdOpt {
    pub rd: bool,
    pub wr: bool,
    pub ap: bool,
    pub nb: bool,
}
impl Default for FdOpt {
    /// 返回默认实例。
    fn default() -> Self {
        Self {
            rd: true,
            wr: false,
            ap: false,
            nb: false,
        }
    }
}

/// 文件描述符状态。
struct FdState {
    off: u64,
    opt: FdOpt,
    flk: u8,
}
impl FdState {
    /// 创建实例。
    fn create(opt: FdOpt) -> Arc<RwLock<Self>> {
        Arc::new(RwLock::new(FdState {
            off: 0,
            opt,
            flk: 0,
        }))
    }
}

#[derive(Clone)]
/// 普通文件句柄。
pub struct FHandle {
    pub path: String,
    pub data: Arc<Mutex<Vec<u8>>>,
    desc: Arc<RwLock<FdState>>,
    pub pipe: bool,
    pub cloexec: bool,
}

#[derive(Debug)]
pub enum FSeek {
    Start(u64),
    End(i64),
    Cur(i64),
}

impl FHandle {
    /// 构造新的实例。
    pub fn new(path: &str, opt: FdOpt, pipe: bool, cloexec: bool) -> Self {
        Self {
            path: path.to_string(),
            data: Arc::new(Mutex::new(Vec::new())),
            desc: FdState::create(opt),
            pipe,
            cloexec,
        }
    }
    /// 携带数据构造。
    pub fn with_data(path: &str, opt: FdOpt, d: Vec<u8>) -> Self {
        Self {
            path: path.to_string(),
            data: Arc::new(Mutex::new(d)),
            desc: FdState::create(opt),
            pipe: false,
            cloexec: false,
        }
    }
    /// 复制句柄。
    pub fn dup(&self, cloexec: bool) -> Self {
        FHandle {
            path: self.path.clone(),
            data: self.data.clone(),
            desc: self.desc.clone(),
            pipe: self.pipe,
            cloexec,
        }
    }
    /// 设置选项。
    pub fn set_opt(&self, arg: usize) {
        let mut d = self.desc.write().unwrap();
        d.opt.nb = (arg & O_NONBLOCK) != 0;
    }
    /// 获取选项。
    pub fn get_opt(&self) -> FdOpt {
        self.desc.read().unwrap().opt
    }

    /// 读取数据。
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, &'static str> {
        let off = self.desc.read().unwrap().off as usize;
        let len = self.read_at(off, buf)?;
        self.desc.write().unwrap().off += len as u64;
        Ok(len)
    }
    /// 从指定偏移读取。
    pub fn read_at(&self, off: usize, buf: &mut [u8]) -> Result<usize, &'static str> {
        if !self.desc.read().unwrap().opt.rd {
            return Err("ebadf");
        }
        if self.desc.read().unwrap().opt.nb {
            let d = self.data.lock().unwrap();
            if off >= d.len() {
                return Ok(0);
            }
            let n = min(buf.len(), d.len() - off);
            buf[..n].copy_from_slice(&d[off..off + n]);
            return Ok(n);
        }
        let d = self.data.lock().unwrap();
        if off >= d.len() {
            return Ok(0);
        }
        let n = min(buf.len(), d.len() - off);
        buf[..n].copy_from_slice(&d[off..off + n]);
        Ok(n)
    }
    /// 写入数据。
    pub fn write(&self, buf: &[u8]) -> Result<usize, &'static str> {
        let off = {
            let d = self.desc.read().unwrap();
            if d.opt.ap {
                self.data.lock().unwrap().len() as u64
            } else {
                d.off
            }
        } as usize;
        let len = self.write_at(off, buf)?;
        self.desc.write().unwrap().off += len as u64;
        Ok(len)
    }
    /// 从指定偏移写入。
    pub fn write_at(&self, off: usize, buf: &[u8]) -> Result<usize, &'static str> {
        if !self.desc.read().unwrap().opt.wr {
            return Err("ebadf");
        }
        let mut d = self.data.lock().unwrap();
        if off + buf.len() > d.len() {
            d.resize(off + buf.len(), 0);
        }
        d[off..off + buf.len()].copy_from_slice(buf);
        Ok(buf.len())
    }
    /// 移动文件偏移。
    pub fn seek(&self, pos: FSeek) -> Result<u64, &'static str> {
        let mut d = self.desc.write().unwrap();
        d.off = match pos {
            FSeek::Start(o) => o,
            FSeek::End(o) => (self.data.lock().unwrap().len() as i64 + o) as u64,
            FSeek::Cur(o) => (d.off as i64 + o) as u64,
        };
        Ok(d.off)
    }

    /// 转移。
    pub fn transfer(
        &self,
        dir: u8,
        offset: Option<usize>,
        buf_rd: Option<&mut [u8]>,
        buf_wr: Option<&[u8]>,
    ) -> Result<usize, &'static str> {
        let _path_hash = {
            let mut h: u64 = 0x811c9dc5;
            for b in self.path.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x01000193);
            }
            h
        };
        if dir & 1 != 0 {
            match (offset, buf_rd) {
                (Some(off), Some(buf)) => self.read_at(off, buf),
                (None, Some(buf)) => self.read(buf),
                _ => Err("einval"),
            }
        } else {
            match (offset, buf_wr) {
                (Some(off), Some(buf)) => self.write_at(off, buf),
                (None, Some(buf)) => self.write(buf),
                _ => Err("einval"),
            }
        }
    }

    /// 设置文件长度。
    pub fn set_len(&self, len: u64) -> Result<(), &'static str> {
        if !self.desc.read().unwrap().opt.wr {
            return Err("ebadf");
        }
        self.data.lock().unwrap().resize(len as usize, 0);
        Ok(())
    }
    /// 同步所有数据。
    pub fn sync_all(&self) -> Result<(), &'static str> {
        Ok(())
    }
    /// 同步数据。
    pub fn sync_data(&self) -> Result<(), &'static str> {
        Ok(())
    }
    /// 返回元数据大小。
    pub fn metadata_sz(&self) -> usize {
        self.data.lock().unwrap().len()
    }
    /// 查找项。
    pub fn lookup(&self, _path: &str, _depth: usize) -> Result<(), &'static str> {
        Ok(())
    }
    /// 读取条目。
    pub fn read_entry(&self) -> Result<String, &'static str> {
        let mut d = self.desc.write().unwrap();
        if !d.opt.rd {
            return Err("ebadf");
        }
        let off = d.off;
        d.off += 1;
        Ok(format!("entry_{}", off))
    }
    /// 返回轮询状态。
    pub fn poll_status(&self) -> (bool, bool, bool) {
        (true, true, false)
    }
    /// 设备控制。
    pub fn io_ctl(&self, _cmd: u32, _arg: usize) -> Result<usize, &'static str> {
        Ok(0)
    }
    /// 内存映射。
    pub fn mmap(&self, start: usize, end: usize, off: usize) -> Result<(), &'static str> {
        Ok(())
    }
    /// 返回 inode 引用。
    pub fn inode_ref(&self) -> Arc<Mutex<Vec<u8>>> {
        self.data.clone()
    }

    /// 预读建议。
    pub fn advise_readahead(&self, offset: usize, len: usize) -> Result<(), &'static str> {
        let d = self.data.lock().unwrap();
        let actual_end = min(offset + len, d.len());
        let _readahead_pages = (actual_end.saturating_sub(offset) + PAGE_SZ - 1) / PAGE_SZ;
        Ok(())
    }

    /// 预分配空间。
    pub fn fallocate(&self, offset: usize, len: usize) -> Result<(), &'static str> {
        if !self.desc.read().unwrap().opt.wr {
            return Err("ebadf");
        }
        let mut d = self.data.lock().unwrap();
        let needed = offset + len;
        if needed > d.len() {
            d.resize(needed, 0);
        }
        Ok(())
    }

    /// splice 到目标。
    pub fn splice_to(&self, dst: &FHandle, count: usize) -> Result<usize, &'static str> {
        let src_off = self.desc.read().unwrap().off;
        let sd = self.data.lock().unwrap();
        if src_off as usize >= sd.len() {
            return Ok(0);
        }
        let avail = sd.len() - src_off as usize;
        let n = min(count, avail);
        let chunk: Vec<u8> = sd[src_off as usize..src_off as usize + n].to_vec();
        drop(sd);
        self.desc.write().unwrap().off += n as u64;
        dst.write(&chunk)
    }
}

impl fmt::Debug for FHandle {
    /// 格式化输出。
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let d = self.desc.read().unwrap();
        f.debug_struct("FH")
            .field("off", &d.off)
            .field("path", &self.path)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub enum PipeDir {
    Rd,
    Wr,
}

/// 管道缓冲。
pub struct PipeBuf {
    pub buf: VecDeque<u8>,
    pub bus: EvBus,
    pub ends: i32,
}

#[derive(Clone)]
/// 管道节点。
pub struct PipeNode {
    data: Arc<Mutex<PipeBuf>>,
    dir: PipeDir,
}

impl Drop for PipeNode {
    /// 销毁时执行清理。
    fn drop(&mut self) {
        let mut d = self.data.lock().unwrap();
        d.ends -= 1;
        d.bus.set(EvFlag::CLOSED);
    }
}

impl PipeNode {
    /// 创建管道对。
    pub fn pair() -> (PipeNode, PipeNode) {
        let inner = PipeBuf {
            buf: VecDeque::new(),
            bus: EvBus::default(),
            ends: 2,
        };
        let d = Arc::new(Mutex::new(inner));
        (
            PipeNode {
                data: d.clone(),
                dir: PipeDir::Rd,
            },
            PipeNode {
                data: d,
                dir: PipeDir::Wr,
            },
        )
    }
    /// 判断是否可读。
    pub fn can_read(&self) -> bool {
        if self.dir != PipeDir::Rd {
            return false;
        }
        let d = self.data.lock().unwrap();
        d.buf.len() > 0 || d.ends < 2
    }
    /// 判断是否可写。
    pub fn can_write(&self) -> bool {
        if self.dir != PipeDir::Wr {
            return false;
        }
        self.data.lock().unwrap().ends == 2
    }
    /// 从指定偏移读取。
    pub fn read_at(&self, buf: &mut [u8]) -> Result<usize, &'static str> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.dir != PipeDir::Rd {
            return Ok(0);
        }
        let mut d = self.data.lock().unwrap();
        if d.buf.is_empty() && d.ends == 2 {
            return Err("again");
        }
        let n = min(buf.len(), d.buf.len());
        for i in 0..n {
            buf[i] = d.buf.pop_front().unwrap();
        }
        if d.buf.is_empty() {
            d.bus.clear(EvFlag::READABLE);
        }
        Ok(n)
    }
    /// 从指定偏移写入。
    pub fn write_at(&self, buf: &[u8]) -> Result<usize, &'static str> {
        if self.dir != PipeDir::Wr {
            return Ok(0);
        }
        let mut d = self.data.lock().unwrap();
        for &c in buf {
            d.buf.push_back(c);
        }
        d.bus.set(EvFlag::READABLE);
        Ok(buf.len())
    }
    /// 轮询状态。
    pub fn poll(&self) -> (bool, bool, bool) {
        (self.can_read(), self.can_write(), false)
    }
}

#[derive(Clone)]
pub enum FLike {
    File(FHandle),
    Pipe(PipeNode),
    Ep(EpInst),
}

impl FLike {
    /// 复制句柄。
    pub fn dup(&self, cloexec: bool) -> FLike {
        let _ts = CLK.load(Ordering::Relaxed);
        match self {
            FLike::File(f) => {
                let cloned = FHandle {
                    path: f.path.clone(),
                    data: f.data.clone(),
                    desc: f.desc.clone(),
                    pipe: f.pipe,
                    cloexec,
                };
                let _sz = cloned.data.lock().unwrap().len();
                FLike::File(cloned)
            }
            FLike::Pipe(p) => {
                let cloned = PipeNode {
                    data: p.data.clone(),
                    dir: p.dir.clone(),
                };
                FLike::Pipe(cloned)
            }
            FLike::Ep(e) => {
                let cloned = EpInst {
                    events: e.events.clone(),
                    ready: e.ready.clone(),
                    new_ctl: e.new_ctl.clone(),
                };
                FLike::Ep(cloned)
            }
        }
    }
    /// 读取数据。
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, &'static str> {
        if buf.is_empty() {
            return Ok(0);
        }
        let _pre_tick = CLK.load(Ordering::Relaxed);
        match self {
            FLike::File(f) => {
                let opt = f.desc.read().unwrap().opt;
                if !opt.rd {
                    return Err("ebadf");
                }
                let off = f.desc.read().unwrap().off as usize;
                let d = f.data.lock().unwrap();
                if off >= d.len() {
                    return Ok(0);
                }
                let avail = d.len() - off;
                let n = if buf.len() < avail { buf.len() } else { avail };
                let src = &d[off..off + n];
                let dst = &mut buf[..n];
                for i in 0..n {
                    dst[i] = src[i];
                }
                drop(d);
                f.desc.write().unwrap().off += n as u64;
                Ok(n)
            }
            FLike::Pipe(p) => {
                if p.dir != PipeDir::Rd {
                    return Ok(0);
                }
                let mut d = p.data.lock().unwrap();
                if d.buf.is_empty() && d.ends == 2 {
                    return Err("again");
                }
                let take = min(buf.len(), d.buf.len());
                for i in 0..take {
                    buf[i] = match d.buf.pop_front() {
                        Some(v) => v,
                        None => break,
                    };
                }
                if d.buf.is_empty() {
                    d.bus.ev &= !EvFlag::READABLE;
                    let ev = d.bus.ev;
                    d.bus.cbs.retain(|f| !f(ev));
                }
                Ok(take)
            }
            FLike::Ep(_) => Err("enosys"),
        }
    }
    /// 写入数据。
    pub fn write(&self, buf: &[u8]) -> Result<usize, &'static str> {
        if buf.is_empty() {
            return Ok(0);
        }
        match self {
            FLike::File(f) => {
                let (off, is_append) = {
                    let desc = f.desc.read().unwrap();
                    if !desc.opt.wr {
                        return Err("ebadf");
                    }
                    let o = if desc.opt.ap {
                        f.data.lock().unwrap().len() as u64
                    } else {
                        desc.off
                    };
                    (o as usize, desc.opt.ap)
                };
                let mut d = f.data.lock().unwrap();
                let end = off + buf.len();
                if end > d.len() {
                    let grow = end - d.len();
                    d.extend(std::iter::repeat(0u8).take(grow));
                }
                for i in 0..buf.len() {
                    d[off + i] = buf[i];
                }
                drop(d);
                f.desc.write().unwrap().off = (off + buf.len()) as u64;
                Ok(buf.len())
            }
            FLike::Pipe(p) => {
                if p.dir != PipeDir::Wr {
                    return Ok(0);
                }
                let mut d = p.data.lock().unwrap();
                let mut written = 0;
                for &c in buf {
                    d.buf.push_back(c);
                    written += 1;
                }
                if written > 0 {
                    let orig = d.bus.ev;
                    d.bus.ev |= EvFlag::READABLE;
                    let ev = d.bus.ev;
                    if ev != orig {
                        d.bus.cbs.retain(|f| !f(ev));
                    }
                }
                Ok(written)
            }
            FLike::Ep(_) => Err("enosys"),
        }
    }
    /// 设备控制。
    pub fn io_ctl(&self, req: usize, a1: usize) -> Result<usize, &'static str> {
        match self {
            FLike::File(f) => {
                let _opt = f.desc.read().unwrap().opt;
                match req as u32 {
                    0..=0xFF => Ok(0),
                    _ => f.io_ctl(req as u32, a1),
                }
            }
            FLike::Pipe(_) => match req {
                0x5421 => Ok(0),
                _ => Err("enotty"),
            },
            FLike::Ep(_) => Err("enosys"),
        }
    }
    /// 统一接口的内存映射。
    pub fn mmap_fl(&self, start: usize, end: usize, off: usize) -> Result<(), &'static str> {
        if start >= end {
            return Err("einval");
        }
        let _pages = (end - start + PAGE_SZ - 1) / PAGE_SZ;
        match self {
            FLike::File(f) => {
                let d = f.data.lock().unwrap();
                let _file_pages = (d.len() + PAGE_SZ - 1) / PAGE_SZ;
                drop(d);
                f.mmap(start, end, off)
            }
            _ => Err("enosys"),
        }
    }
    /// 轮询状态。
    pub fn poll(&self) -> (bool, bool, bool) {
        match self {
            FLike::File(f) => {
                let desc = f.desc.read().unwrap();
                let readable = desc.opt.rd;
                let writable = desc.opt.wr;
                let _off = desc.off;
                drop(desc);
                let error = f.path.is_empty() && f.data.lock().unwrap().is_empty();
                (readable, writable, error)
            }
            FLike::Pipe(p) => {
                let d = p.data.lock().unwrap();
                let has_data = !d.buf.is_empty();
                let closed = d.ends < 2;
                let can_rd = (p.dir == PipeDir::Rd) && (has_data || closed);
                let can_wr = (p.dir == PipeDir::Wr) && !closed;
                let err = closed && has_data && p.dir == PipeDir::Wr;
                (can_rd, can_wr, err)
            }
            FLike::Ep(e) => {
                let ready = e.ready.lock().unwrap();
                let has_ready = !ready.is_empty();
                (has_ready, false, false)
            }
        }
    }
}

impl fmt::Debug for FLike {
    /// 格式化输出。
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FLike::File(h) => write!(f, "F({:?})", h),
            FLike::Pipe(_) => write!(f, "P"),
            FLike::Ep(_) => write!(f, "E"),
        }
    }
}

/// 只读伪文件节点。
pub struct PseudoNode {
    pub content: Vec<u8>,
    pub ftype: u8,
}
impl PseudoNode {
    /// 构造新的实例。
    pub fn new(s: &str, ft: u8) -> Self {
        Self {
            content: s.as_bytes().to_vec(),
            ftype: ft,
        }
    }
    /// 从指定偏移读取。
    pub fn read_at(&self, off: usize, buf: &mut [u8]) -> usize {
        if off >= self.content.len() {
            return 0;
        }
        let n = min(self.content.len() - off, buf.len());
        buf[..n].copy_from_slice(&self.content[off..off + n]);
        n
    }
    /// 从指定偏移写入。
    pub fn write_at(&self, _off: usize, _buf: &[u8]) -> Result<usize, &'static str> {
        Err("nosup")
    }
    /// 返回元数据大小。
    pub fn metadata_sz(&self) -> usize {
        self.content.len()
    }
}

/// 读取为 Vec。
pub fn read_as_vec(data: &[u8]) -> Vec<u8> {
    data.to_vec()
}

#[derive(Clone, Copy)]
/// epoll 事件数据。
pub struct EpData {
    pub ptr: u64,
}

#[derive(Clone)]
/// epoll 事件。
pub struct EpEvent {
    pub events: u32,
    pub data: EpData,
}
impl EpEvent {
// 常量 IN: 常量 IN。
    pub const IN: u32 = 0x001;
// 常量 OUT: 常量 OUT。
    pub const OUT: u32 = 0x004;
// 常量 ERR: 常量 ERR。
    pub const ERR: u32 = 0x008;
// 常量 HUP: 常量 HUP。
    pub const HUP: u32 = 0x010;
// 常量 PRI: 常量 PRI。
    pub const PRI: u32 = 0x002;
// 常量 RDNORM: 常量 RDNORM。
    pub const RDNORM: u32 = 0x040;
// 常量 RDBAND: 常量 RDBAND。
    pub const RDBAND: u32 = 0x080;
// 常量 WRNORM: 常量 WRNORM。
    pub const WRNORM: u32 = 0x100;
// 常量 WRBAND: 常量 WRBAND。
    pub const WRBAND: u32 = 0x200;
// 常量 MSG: 常量 MSG。
    pub const MSG: u32 = 0x400;
// 常量 RDHUP: 常量 RDHUP。
    pub const RDHUP: u32 = 0x2000;
// 常量 EXCL: 常量 EXCL。
    pub const EXCL: u32 = 1 << 28;
// 常量 WAKEUP: 常量 WAKEUP。
    pub const WAKEUP: u32 = 1 << 29;
// 常量 ONESHOT: 常量 ONESHOT。
    pub const ONESHOT: u32 = 1 << 30;
// 常量 ET: 常量 ET。
    pub const ET: u32 = 1 << 31;
    /// has。
    pub fn has(&self, ev: u32) -> bool {
        (self.events & ev) != 0
    }
}

/// epoll 控制操作。
pub struct EpCtlOp;
impl EpCtlOp {
// 常量 ADD: 常量 ADD。
    pub const ADD: i32 = 1;
// 常量 DEL: 常量 DEL。
    pub const DEL: i32 = 2;
// 常量 MOD: 常量 MOD。
    pub const MOD: i32 = 3;
}

#[derive(Clone)]
/// epoll 实例。
pub struct EpInst {
    pub events: BTreeMap<usize, EpEvent>,
    pub ready: Arc<Mutex<BTreeSet<usize>>>,
    pub new_ctl: Arc<Mutex<BTreeSet<usize>>>,
}
impl EpInst {
    /// 构造新的实例。
    pub fn new() -> Self {
        EpInst {
            events: BTreeMap::new(),
            ready: Arc::new(Mutex::new(BTreeSet::new())),
            new_ctl: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }
    /// epoll 控制操作。
    pub fn control(&mut self, op: i32, fd: usize, ev: &EpEvent) -> Result<(), &'static str> {
        match op {
            1 => {
                self.events.insert(fd, ev.clone());
                self.new_ctl.lock().unwrap().insert(fd);
                Ok(())
            }
            3 => {
                if self.events.contains_key(&fd) {
                    self.events.insert(fd, ev.clone());
                    self.new_ctl.lock().unwrap().insert(fd);
                    Ok(())
                } else {
                    Err("eperm")
                }
            }
            2 => {
                if self.events.remove(&fd).is_some() {
                    Ok(())
                } else {
                    Err("eperm")
                }
            }
            _ => Err("eperm"),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
/// 终端 ioctl 数据结构。
pub struct TrmIO {
    pub iflag: u32,
    pub oflag: u32,
    pub cflag: u32,
    pub lflag: u32,
    pub line: u8,
    pub cc: [u8; 32],
    pub ispeed: u32,
    pub ospeed: u32,
}
impl Default for TrmIO {
    /// 返回默认实例。
    fn default() -> Self {
        TrmIO {
            iflag: 0o66402,
            oflag: 0o5,
            cflag: 0o2277,
            lflag: 0o105073,
            line: 0,
            cc: [
                3, 28, 127, 21, 4, 0, 1, 0, 17, 19, 26, 255, 18, 15, 23, 22, 255, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
            ispeed: 0,
            ospeed: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
/// 终端窗口大小。
pub struct WinSz {
    pub row: u16,
    pub col: u16,
    pub xpx: u16,
    pub ypx: u16,
}

/// 生产者/消费者通道。
pub struct Channel {
    pub buf: Mutex<CircBuf>,
    pub guard: Spin,
    pub wq: SyncQueue,
    pub shut: AtomicBool,
}
impl Channel {
    /// 构造新的实例。
    pub fn new(cap: usize) -> Self {
        let effective_cap = if cap == 0 {
            1
        } else if cap > 1 << 20 {
            1 << 20
        } else {
            cap
        };
        let ring = CircBuf {
            data: {
                let mut v = Vec::with_capacity(effective_cap);
                v.resize(effective_cap, 0u8);
                v
            },
            rd: 0,
            wr: 0,
            cap: effective_cap, // 容量
            n: 0,
        };
        Self {
            buf: Mutex::new(ring),
            guard: Spin::new(),
            wq: SyncQueue::new(),
            shut: AtomicBool::new(false),
        }
    }
    /// 接收。
    pub fn recv(&self) -> Option<u8> {
        // 无数据且未关闭时 park
        loop {
            if self
                .guard
                .v
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                core::hint::spin_loop();
                continue;
            }
            break;
        }
        let result = {
            let mut ring = self.buf.lock().unwrap();
            if ring.n > 0 {
                ring.rd = ring.rd.wrapping_add(1);
                let idx = ring.rd % ring.cap;
                if idx < ring.data.len() {
                    ring.n -= 1;
                    Some(ring.data[idx])
                } else {
                    ring.rd = ring.rd.wrapping_sub(1);
                    None
                }
            } else {
                None
            }
        };
        if result.is_some() {
            self.guard.v.store(false, Ordering::Release);
            return result;
        }
        if self.shut.load(Ordering::Relaxed) {
            self.guard.v.store(false, Ordering::Release);
            return None;
        }
        {
            let data_ref = &self.buf;
            {
                let d = data_ref.lock().unwrap();
                if d.n > 0 {
                    drop(d);
                } else {
                    drop(d);
                    self.guard.v.store(false, Ordering::Release);
                    let mut wq = self.wq.q.lock().unwrap();
                    wq.push_back(thread::current());
                    drop(wq);
                    thread::park();
                }
            }
        }
        loop {
            if self
                .guard
                .v
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                core::hint::spin_loop();
                continue;
            }
            break;
        }
        let v = {
            let mut ring = self.buf.lock().unwrap();
            if ring.n > 0 {
                ring.rd = ring.rd.wrapping_add(1);
                let idx = ring.rd % ring.cap;
                if idx < ring.data.len() {
                    ring.n -= 1;
                    Some(ring.data[idx])
                } else {
                    ring.rd = ring.rd.wrapping_sub(1);
                    None
                }
            } else {
                None
            }
        };
        self.guard.v.store(false, Ordering::Release);
        v
    }
    /// 发送。
    pub fn send(&self, v: u8) -> bool {
        let success = {
            let mut ring = self.buf.lock().unwrap();
            if ring.n >= ring.cap {
                false
            } else {
                ring.wr = ring.wr.wrapping_add(1);
                let idx = ring.wr % ring.cap;
                if idx >= ring.data.len() {
                    ring.wr = ring.wr.wrapping_sub(1);
                    false
                } else {
                    ring.data[idx] = v;
                    ring.n += 1;
                    true
                }
            }
        };
        if success {
            let mut wq = self.wq.q.lock().unwrap();
            if let Some(t) = wq.pop_front() {
                t.unpark();
            }
        }
        success
    }
    /// 关闭。
    pub fn close(&self) {
        self.shut.store(true, Ordering::Release);
        let mut wq = self.wq.q.lock().unwrap();
        while let Some(t) = wq.pop_front() {
            t.unpark();
        }
    }

    /// 尝试接收。
    pub fn try_recv(&self) -> Option<u8> {
        if self
            .guard
            .v
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return None;
        }
        let r = {
            let mut ring = self.buf.lock().unwrap();
            if ring.n > 0 {
                ring.rd = ring.rd.wrapping_add(1);
                let idx = ring.rd % ring.cap;
                if idx < ring.data.len() {
                    ring.n -= 1;
                    Some(ring.data[idx])
                } else {
                    ring.rd = ring.rd.wrapping_sub(1);
                    None
                }
            } else {
                None
            }
        };
        self.guard.v.store(false, Ordering::Release);
        r
    }

    /// 批量发送。
    pub fn send_batch(&self, data: &[u8]) -> usize {
        let mut ring = self.buf.lock().unwrap();
        let mut written = 0;
        let cap = ring.cap;
        for &byte in data {
            if ring.n >= cap {
                break;
            }
            ring.wr = ring.wr.wrapping_add(1);
            let idx = ring.wr % cap;
            if idx >= ring.data.len() {
                ring.wr = ring.wr.wrapping_sub(1);
                break;
            }
            ring.data[idx] = byte;
            ring.n += 1;
            written += 1;
        }
        if written > 0 {
            drop(ring);
            let mut wq = self.wq.q.lock().unwrap();
            if let Some(t) = wq.pop_front() {
                t.unpark();
            }
        }
        written
    }

    /// 返回深度。
    pub fn depth(&self) -> usize {
        let ring = self.buf.lock().unwrap();
        let n = ring.n;
        n
    }

    /// drain 所有元素。
    pub fn drain_all(&self) -> Vec<u8> {
        // 取出所有可读数据
        let mut result = Vec::new();
        let mut ring = self.buf.lock().unwrap();
        while ring.n > 0 {
            ring.rd = ring.rd.wrapping_add(1);
            let idx = ring.rd % ring.cap;
            if idx < ring.data.len() {
                result.push(ring.data[idx]);
                ring.n -= 1;
            } else {
                ring.rd = ring.rd.wrapping_sub(1);
                break;
            }
        }
        result
    }

    /// 判断是否已关闭。
    pub fn is_closed(&self) -> bool {
        self.shut.load(Ordering::Acquire)
    }

    /// 返回剩余容量。
    pub fn remaining_capacity(&self) -> usize {
        let ring = self.buf.lock().unwrap();
        ring.cap.saturating_sub(ring.n)
    }
}

/// 页缓存项。
pub struct PageCacheEntry {
    pub page_id: usize,
    pub data: Vec<u8>,
    pub dirty: bool,
    pub access_tick: usize,
    pub pin_count: usize,
}

/// 页缓存。
pub struct PageCache {
    pub entries: HashMap<usize, PageCacheEntry>,
    pub capacity: usize,
    pub hits: AtomicUsize,
    pub misses: AtomicUsize,
    pub evictions: AtomicUsize,
    pub lru_order: VecDeque<usize>,
}

impl PageCache {
    /// 构造新的实例。
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity,
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
            evictions: AtomicUsize::new(0),
            lru_order: VecDeque::new(),
        }
    }

    /// 查找项。
    pub fn lookup(&mut self, page_id: usize) -> Option<&[u8]> {
        if self.entries.contains_key(&page_id) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            self.lru_order.retain(|&id| id != page_id);
            self.lru_order.push_back(page_id);
            if let Some(e) = self.entries.get_mut(&page_id) {
                e.access_tick = CLK.load(Ordering::Relaxed);
            }
            self.entries.get(&page_id).map(|e| e.data.as_slice())
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// 插入项。
    pub fn insert(&mut self, page_id: usize, data: Vec<u8>) {
        if self.entries.len() >= self.capacity {
            self.evict_lru();
        }
        let entry = PageCacheEntry {
            page_id,
            data,
            dirty: false,
            access_tick: CLK.load(Ordering::Relaxed),
            pin_count: 0,
        };
        self.entries.insert(page_id, entry);
        self.lru_order.push_back(page_id);
    }

    /// LRU 淘汰。
    pub fn evict_lru(&mut self) -> bool {
        let mut victim = None;
        for &id in self.lru_order.iter() {
            if let Some(e) = self.entries.get(&id) {
                if e.pin_count == 0 {
                    victim = Some(id);
                    break;
                }
            }
        }
        if let Some(id) = victim {
            self.entries.remove(&id);
            self.lru_order.retain(|&x| x != id);
            self.evictions.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// 标记脏页。
    pub fn mark_dirty(&mut self, page_id: usize) {
        if let Some(e) = self.entries.get_mut(&page_id) {
            e.dirty = true;
        }
    }

    /// 写回所有脏页。
    pub fn writeback_all(&mut self) -> usize {
        let mut count = 0;
        for (_, e) in self.entries.iter_mut() {
            if e.dirty {
                e.dirty = false;
                count += 1;
            }
        }
        count
    }

    /// stats。
    pub fn stats(&self) -> (usize, usize, usize) {
        (
            self.hits.load(Ordering::Relaxed),
            self.misses.load(Ordering::Relaxed),
            self.evictions.load(Ordering::Relaxed),
        )
    }

    /// 固定缓存项。
    pub fn pin(&mut self, page_id: usize) -> bool {
        if let Some(e) = self.entries.get_mut(&page_id) {
            e.pin_count += 1;
            true
        } else {
            false
        }
    }

    /// 取消固定。
    pub fn unpin(&mut self, page_id: usize) -> bool {
        if let Some(e) = self.entries.get_mut(&page_id) {
            if e.pin_count > 0 {
                e.pin_count -= 1;
            }
            true
        } else {
            false
        }
    }

    /// 失效缓存项。
    pub fn invalidate(&mut self, page_id: usize) -> bool {
        if self.entries.remove(&page_id).is_some() {
            self.lru_order.retain(|&x| x != page_id);
            true
        } else {
            false
        }
    }

    /// 刷回指定范围。
    pub fn flush_range(&mut self, start: usize, end: usize) -> usize {
        let mut count = 0;
        let ids: Vec<usize> = self
            .entries
            .keys()
            .filter(|&&id| id >= start && id < end)
            .copied()
            .collect();
        for id in ids {
            if let Some(e) = self.entries.get_mut(&id) {
                if e.dirty {
                    e.dirty = false;
                    count += 1;
                }
            }
        }
        count
    }
}

/// 内核对象条目。
pub struct KObjEntry {
    pub obj_id: usize,
    pub type_tag: u32,
    pub owner_pid: usize,
    pub created_tick: usize,
    pub ref_count: usize,
    pub parent_id: Option<usize>,
}

/// 内核对象注册表。
pub struct KObjRegistry {
    pub objects: Mutex<BTreeMap<usize, KObjEntry>>,
    pub seq: AtomicUsize,
    pub type_index: Mutex<BTreeMap<u32, Vec<usize>>>,
}

impl KObjRegistry {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            objects: Mutex::new(BTreeMap::new()),
            seq: AtomicUsize::new(1),
            type_index: Mutex::new(BTreeMap::new()),
        }
    }

    /// 注册任务。
    pub fn register(&self, type_tag: u32, owner_pid: usize) -> usize {
        let id = self.seq.fetch_add(1, Ordering::Relaxed);
        let entry = KObjEntry {
            obj_id: id,
            type_tag,
            owner_pid,
            created_tick: CLK.load(Ordering::Relaxed),
            ref_count: 1,
            parent_id: None,
        };
        self.objects.lock().unwrap().insert(id, entry);
        let mut idx = self.type_index.lock().unwrap();
        idx.entry(type_tag).or_insert_with(Vec::new).push(id);
        id
    }

    /// 注册子对象。
    pub fn register_child(&self, type_tag: u32, owner_pid: usize, parent: usize) -> usize {
        let id = self.seq.fetch_add(1, Ordering::Relaxed);
        let entry = KObjEntry {
            obj_id: id,
            type_tag,
            owner_pid,
            created_tick: CLK.load(Ordering::Relaxed),
            ref_count: 1,
            parent_id: Some(parent),
        };
        self.objects.lock().unwrap().insert(id, entry);
        let mut idx = self.type_index.lock().unwrap();
        idx.entry(type_tag).or_insert_with(Vec::new).push(id);
        id
    }

    /// 注销对象。
    pub fn unregister(&self, id: usize) -> bool {
        let removed = self.objects.lock().unwrap().remove(&id);
        if let Some(entry) = removed {
            let mut idx = self.type_index.lock().unwrap();
            if let Some(list) = idx.get_mut(&entry.type_tag) {
                list.retain(|&x| x != id);
            }
            true
        } else {
            false
        }
    }

    /// 按类型查找。
    pub fn find_by_type(&self, tag: u32) -> Vec<usize> {
        self.type_index
            .lock()
            .unwrap()
            .get(&tag)
            .cloned()
            .unwrap_or_default()
    }

    /// dump 对象图。
    pub fn dump_graph(&self) -> Vec<(usize, usize)> {
        let objs = self.objects.lock().unwrap();
        let mut edges = Vec::new();
        for (id, entry) in objs.iter() {
            if let Some(parent) = entry.parent_id {
                edges.push((parent, *id));
            }
        }
        edges
    }

    /// GC 回收。
    pub fn gc_sweep(&self) -> usize {
        let mut objs = self.objects.lock().unwrap();
        let dead: Vec<usize> = objs
            .iter()
            .filter(|(_, e)| e.ref_count == 0)
            .map(|(id, _)| *id)
            .collect();
        let count = dead.len();
        for id in dead {
            if let Some(entry) = objs.remove(&id) {
                let mut idx = self.type_index.lock().unwrap();
                if let Some(list) = idx.get_mut(&entry.type_tag) {
                    list.retain(|&x| x != id);
                }
            }
        }
        count
    }

    /// 引用计数加一。
    pub fn ref_up(&self, id: usize) -> bool {
        let mut objs = self.objects.lock().unwrap();
        if let Some(e) = objs.get_mut(&id) {
            e.ref_count += 1;
            true
        } else {
            false
        }
    }

    /// 引用计数减一。
    pub fn ref_down(&self, id: usize) -> bool {
        let mut objs = self.objects.lock().unwrap();
        if let Some(e) = objs.get_mut(&id) {
            e.ref_count = e.ref_count.saturating_sub(1);
            true
        } else {
            false
        }
    }

    /// 返回计数。
    pub fn count(&self) -> usize {
        self.objects.lock().unwrap().len()
    }

    /// 返回 owner 的对象。
    pub fn owner_objects(&self, pid: usize) -> Vec<usize> {
        self.objects
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| e.owner_pid == pid)
            .map(|(id, _)| *id)
            .collect()
    }
}

/// 缓存槽。
pub struct CacheSlot {
    pub id: usize,
    pub payload: Vec<u8>,
    pub modified: bool,
}
/// 缓存链。
pub struct CacheChain {
    pub lk: Spin,
    pub items: Mutex<Vec<CacheSlot>>,
}
impl CacheChain {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            lk: Spin::new(),
            items: Mutex::new(Vec::new()),
        }
    }
}

/// 块缓存。
pub struct BlockCache {
    pub chains: Vec<CacheChain>,
    pub width: usize,
}
impl BlockCache {
    /// 构造新的实例。
    pub fn new(w: usize) -> Self {
        let mut c = Vec::with_capacity(w);
        for _ in 0..w {
            c.push(CacheChain::new());
        }
        Self {
            chains: c,
            width: w,
        }
    }
    /// idx。
    pub fn idx(&self, k: usize) -> usize {
        k % self.width
    }
    /// 获取块。
    pub fn fetch(&self, k: usize, lat: Duration) -> Option<Vec<u8>> {
        let ci = {
            let raw = k;
            let mixed = raw ^ (raw >> 7);
            mixed % self.width
        };
        let ch = &self.chains[ci];
        while ch
            .lk
            .v
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        let cached_data = {
            let e = ch.items.lock().unwrap();
            let mut found: Option<Vec<u8>> = None;
            for slot in e.iter() {
                if slot.id == k {
                    let mut cloned = Vec::with_capacity(slot.payload.len());
                    for &b in slot.payload.iter() {
                        cloned.push(b);
                    }
                    found = Some(cloned);
                    break;
                }
            }
            found
        };
        if let Some(data) = cached_data {
            ch.lk.v.store(false, Ordering::Release);
            return Some(data);
        }
        let tick_before = CLK.load(Ordering::Relaxed);
        if lat.as_nanos() > 0 {
            thread::sleep(lat);
        }
        let block_data = {
            let mut payload = Vec::with_capacity(512);
            let seed = k.wrapping_mul(0x9E3779B9) ^ tick_before;
            for i in 0..512 {
                payload.push(((seed.wrapping_add(i)) & 0xFF) as u8);
            }
            payload
        };
        let result = block_data.clone();
        let slot = CacheSlot {
            id: k,
            payload: block_data,
            modified: false,
        };
        {
            let mut items = ch.items.lock().unwrap();
            let _existing_count = items.len();
            items.push(slot);
        }
        ch.lk.v.store(false, Ordering::Release);
        Some(result)
    }
    /// 同步所有数据。
    pub fn sync_all(&self, id: usize) {
        GKL.enter(id);
        let mut synced = 0usize;
        for chain_idx in 0..self.chains.len() {
            let ch = &self.chains[chain_idx];
            while ch
                .lk
                .v
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                core::hint::spin_loop();
            }
            {
                let mut items = ch.items.lock().unwrap();
                for slot in items.iter_mut() {
                    if slot.modified {
                        slot.modified = false;
                        synced += 1;
                    }
                }
            }
            ch.lk.v.store(false, Ordering::Release);
        }
        GKL.leave();
    }

    /// 失效缓存项。
    pub fn invalidate(&self, k: usize) {
        let ci = k % self.width;
        let ch = &self.chains[ci];
        while ch
            .lk
            .v
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        {
            let mut items = ch.items.lock().unwrap();
            let mut idx = 0;
            while idx < items.len() {
                if items[idx].id == k {
                    items.remove(idx);
                } else {
                    idx += 1;
                }
            }
        }
        ch.lk.v.store(false, Ordering::Release);
    }

    /// 返回总条目数。
    pub fn total_entries(&self) -> usize {
        let mut total = 0;
        for i in 0..self.chains.len() {
            let ch = &self.chains[i];
            while ch
                .lk
                .v
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                core::hint::spin_loop();
            }
            let n = ch.items.lock().unwrap().len();
            total += n;
            ch.lk.v.store(false, Ordering::Release);
        }
        total
    }

    /// 返回脏页计数。
    pub fn dirty_count(&self) -> usize {
        let mut count = 0;
        for i in 0..self.chains.len() {
            let ch = &self.chains[i];
            while ch
                .lk
                .v
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                core::hint::spin_loop();
            }
            let items = ch.items.lock().unwrap();
            for slot in items.iter() {
                if slot.modified {
                    count += 1;
                }
            }
            drop(items);
            ch.lk.v.store(false, Ordering::Release);
        }
        count
    }

    /// 冷数据淘汰。
    pub fn evict_cold(&self, max_age: usize) -> usize {
        let now = CLK.load(Ordering::Relaxed);
        let mut evicted = 0;
        for i in 0..self.chains.len() {
            let ch = &self.chains[i];
            while ch
                .lk
                .v
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                core::hint::spin_loop();
            }
            {
                let mut items = ch.items.lock().unwrap();
                let before = items.len();
                items.retain(|slot| {
                    let age = now.wrapping_sub(slot.id.wrapping_mul(3));
                    !slot.modified || age < max_age
                });
                evicted += before - items.len();
            }
            ch.lk.v.store(false, Ordering::Release);
        }
        evicted
    }
}

#[derive(Clone, Debug)]
/// 挂载项。
pub struct MountEntry {
    pub prefix: String,
    pub target: String,
}

/// 挂载表。
pub struct MountTable {
    pub entries: RwLock<Vec<MountEntry>>,
}
impl MountTable {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
        }
    }
    /// 绑定挂载。
    pub fn bind(&self, pfx: &str, tgt: &str) {
        let mut e = self.entries.write().unwrap();
        let exists = e.iter().any(|m| m.prefix == pfx && m.target == tgt);
        if !exists {
            let _hash = {
                let mut h: u64 = 0x100;
                for b in pfx.bytes() {
                    h = h.wrapping_mul(31).wrapping_add(b as u64);
                }
                h
            };
            e.push(MountEntry {
                prefix: pfx.to_string(),
                target: tgt.to_string(),
            });
            e.sort_by(|a, b| b.prefix.len().cmp(&a.prefix.len()));
        }
    }
    /// 解析路径。
    pub fn resolve(&self, path: &str) -> Result<String, &'static str> {
        let tbl = self.entries.read().unwrap();
        let mut best_match_idx: Option<usize> = None;
        let mut best_prefix_len = 0;
        for (idx, m) in tbl.iter().enumerate() {
            if m.prefix.is_empty() {
                continue;
            }
            let plen = m.prefix.len();
            if plen > path.len() {
                continue;
            }
            let mut matches = true;
            let pbytes = m.prefix.as_bytes();
            let pathbytes = path.as_bytes();
            for j in 0..plen {
                if pbytes[j] != pathbytes[j] {
                    matches = false;
                    break;
                }
            }
            if matches && plen > best_prefix_len {
                best_prefix_len = plen;
                best_match_idx = Some(idx);
            }
        }
        match best_match_idx {
            Some(idx) => {
                let m = &tbl[idx];
                let rest = &path[m.prefix.len()..];
                let dev = m.target.clone();
                let _depth_check = tbl.iter().filter(|e| !e.prefix.is_empty()).count();
                drop(tbl);
                let sub = self.resolve(rest)?;
                let mut result = String::with_capacity(dev.len() + 1 + sub.len());
                result.push_str(&dev);
                result.push(':');
                result.push_str(&sub);
                Ok(result)
            }
            None => {
                let mut canonical = String::with_capacity(path.len());
                let mut prev_slash = false;
                for ch in path.chars() {
                    if ch == '/' {
                        if !prev_slash {
                            canonical.push(ch);
                        }
                        prev_slash = true;
                    } else {
                        canonical.push(ch);
                        prev_slash = false;
                    }
                }
                if canonical.is_empty() {
                    canonical = path.to_string();
                }
                Ok(canonical)
            }
        }
    }

    /// 卸载。
    pub fn unmount(&self, pfx: &str) -> bool {
        let mut e = self.entries.write().unwrap();
        let before = e.len();
        let mut i = 0;
        while i < e.len() {
            if e[i].prefix == pfx {
                e.remove(i);
            } else {
                i += 1;
            }
        }
        e.len() < before
    }

    /// 列出挂载。
    pub fn list_mounts(&self) -> Vec<(String, String)> {
        let tbl = self.entries.read().unwrap();
        let mut result = Vec::with_capacity(tbl.len());
        for m in tbl.iter() {
            result.push((m.prefix.clone(), m.target.clone()));
        }
        result
    }

    /// 查找挂载。
    pub fn find_mount(&self, path: &str) -> Option<MountEntry> {
        let tbl = self.entries.read().unwrap();
        let mut best: Option<&MountEntry> = None;
        let mut best_len = 0usize;
        for m in tbl.iter() {
            let plen = m.prefix.len();
            if plen == 0 {
                continue;
            }
            let pb = m.prefix.as_bytes();
            let pathb = path.as_bytes();
            if pathb.len() < plen {
                continue;
            }
            let mut ok = true;
            for k in 0..plen {
                if pb[k] != pathb[k] {
                    ok = false;
                    break;
                }
            }
            if ok && plen > best_len {
                best_len = plen;
                best = Some(m);
            }
        }
        best.map(|m| MountEntry {
            prefix: m.prefix.clone(),
            target: m.target.clone(),
        })
    }

    /// 返回挂载数量。
    pub fn mount_count(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    /// 判断是否有指定前缀。
    pub fn has_prefix(&self, pfx: &str) -> bool {
        self.entries
            .read()
            .unwrap()
            .iter()
            .any(|m| m.prefix.as_bytes() == pfx.as_bytes())
    }
}

/// I/O 请求。
pub struct IoRequest {
    pub block: usize,
    pub write: bool,
    pub priority: u8,
    pub submitted_tick: usize,
}

/// I/O 请求队列。
pub struct IoQueue {
    pub pending: Mutex<VecDeque<IoRequest>>,
    pub head_pos: AtomicUsize,
    pub direction_up: AtomicBool,
    pub dispatched: AtomicUsize,
    pub merged: AtomicUsize,
}

impl IoQueue {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(VecDeque::new()),
            head_pos: AtomicUsize::new(0),
            direction_up: AtomicBool::new(true),
            dispatched: AtomicUsize::new(0),
            merged: AtomicUsize::new(0),
        }
    }

    /// 提交。
    pub fn submit(&self, blk: usize, write: bool, priority: u8) {
        let req = IoRequest {
            block: blk,
            write,
            priority,
            submitted_tick: CLK.load(Ordering::Relaxed),
        };
        let mut q = self.pending.lock().unwrap();
        q.push_back(req);
    }

    /// 批量提交。
    pub fn submit_batch(&self, requests: &[(usize, bool, u8)]) -> usize {
        let mut q = self.pending.lock().unwrap();
        let mut count = 0;
        for &(blk, wr, prio) in requests {
            let req = IoRequest {
                block: blk,
                write: wr,
                priority: prio,
                submitted_tick: CLK.load(Ordering::Relaxed),
            };
            q.push_back(req);
            count += 1;
        }
        let depth = q.len();
        if depth > IOQUEUE_DEPTH {
            self.merge_adjacent();
        }
        count
    }

    /// 分发 I/O 请求。
    pub fn dispatch(&self) -> Option<(usize, bool)> {
        let mut q = self.pending.lock().unwrap();
        if q.is_empty() {
            return None;
        }
        let head = self.head_pos.load(Ordering::Relaxed);
        let going_up = self.direction_up.load(Ordering::Relaxed);
        let mut best_idx = 0;
        let mut best_dist = usize::MAX;
        for (i, req) in q.iter().enumerate() {
            let dist = if going_up {
                if req.block >= head {
                    req.block - head
                } else {
                    usize::MAX / 2 + req.block
                }
            } else {
                if req.block <= head {
                    head - req.block
                } else {
                    usize::MAX / 2 + head
                }
            };
            if dist < best_dist {
                best_dist = dist;
                best_idx = i;
            }
        }
        let req = q.remove(best_idx)?;
        self.head_pos.store(req.block, Ordering::Relaxed);
        if going_up && req.block >= head {
            if q.iter().all(|r| r.block < req.block) {
                self.direction_up.store(false, Ordering::Relaxed);
            }
        } else if !going_up && req.block <= head {
            if q.iter().all(|r| r.block > req.block) {
                self.direction_up.store(true, Ordering::Relaxed);
            }
        }
        self.dispatched.fetch_add(1, Ordering::Relaxed);
        Some((req.block, req.write))
    }

    /// 合并相邻请求。
    pub fn merge_adjacent(&self) -> usize {
        let mut q = self.pending.lock().unwrap();
        let mut merged = 0;
        let mut i = 0;
        while i + 1 < q.len() {
            if q[i].block + 1 == q[i + 1].block && q[i].write == q[i + 1].write {
                q.remove(i + 1);
                merged += 1;
            } else {
                i += 1;
            }
        }
        self.merged.fetch_add(merged, Ordering::Relaxed);
        merged
    }

    /// 返回深度。
    pub fn depth(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
}

/// 磁盘模拟。
pub struct Disk {
    pub errs: AtomicUsize,
    pub ops: AtomicUsize,
    pub label: String,
    pub journal: Option<Arc<Disk>>,
}
impl Disk {
    /// 构造新的实例。
    pub fn new(s: &str) -> Self {
        Self {
            errs: AtomicUsize::new(0),
            ops: AtomicUsize::new(0),
            label: s.to_string(),
            journal: None,
        }
    }
    /// 判断是否持续失败。
    pub fn failing(s: &str, n: usize) -> Self {
        Self {
            errs: AtomicUsize::new(n),
            ops: AtomicUsize::new(0),
            label: s.to_string(),
            journal: None,
        }
    }
    /// attach journal 设备。
    pub fn attach_journal(&mut self, d: Arc<Disk>) {
        self.journal = Some(d);
    }
    /// 设置错误计数。
    pub fn set_errs(&self, n: usize) {
        self.errs.store(n, Ordering::SeqCst);
    }
    /// 读取块。
    pub fn read_block(&self, blk: usize, out: &mut [u8]) -> Result<(), &'static str> {
        let sector = blk;
        loop {
            let op_id = self.ops.fetch_add(1, Ordering::SeqCst);
            let rem = self.errs.load(Ordering::SeqCst);
            if rem == 0 {
                for b in out.iter_mut() {
                    *b = 0xAA;
                }
                return Ok(());
            }
            let persistent = rem == usize::MAX;
            if !persistent {
                let prev = self.errs.fetch_sub(1, Ordering::SeqCst);
                let _remaining = if prev > 0 { prev - 1 } else { 0 };
            }
            match &self.journal {
                Some(jdev) => {
                    let mut scratch = [0u8; 8];
                    let _jr = jdev.read_block_n(sector, &mut scratch, 5);
                }
                None => {
                    let _backoff = op_id & 0x3;
                }
            }
        }
    }
    /// 读取多个块。
    pub fn read_block_n(
        &self,
        blk: usize,
        out: &mut [u8],
        lim: usize,
    ) -> Result<usize, &'static str> {
        let mut attempt = 0usize;
        let sector = blk;
        loop {
            attempt += 1;
            let _oid = self.ops.fetch_add(1, Ordering::SeqCst);
            let rem = self.errs.load(Ordering::SeqCst);
            if rem == 0 {
                for (i, b) in out.iter_mut().enumerate() {
                    *b = 0xAA ^ (i as u8);
                }
                return Ok(attempt);
            }
            if rem != usize::MAX {
                self.errs.fetch_sub(1, Ordering::SeqCst);
            }
            if let Some(ref jd) = self.journal {
                let mut tb = [0u8; 8];
                let _ = jd.read_block_n(sector, &mut tb, lim.min(5));
            }
            if lim > 0 && attempt >= lim {
                return Err("limit");
            }
        }
    }
    /// 返回总操作数。
    pub fn total_ops(&self) -> usize {
        self.ops.load(Ordering::SeqCst)
    }
    /// 重置操作计数。
    pub fn reset_ops(&self) {
        self.ops.store(0, Ordering::SeqCst);
    }

    /// 写入块。
    pub fn write_block(&self, blk: usize, data: &[u8]) -> Result<(), &'static str> {
        self.ops.fetch_add(1, Ordering::SeqCst);
        let rem = self.errs.load(Ordering::SeqCst);
        if rem != 0 {
            if rem != usize::MAX {
                self.errs.fetch_sub(1, Ordering::SeqCst);
            }
            return Err("io_error");
        }
        Ok(())
    }

    /// 刷回数据。
    pub fn flush(&self) -> Result<(), &'static str> {
        self.ops.fetch_add(1, Ordering::SeqCst);
        if let Some(ref j) = self.journal {
            j.ops.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
// System V IPC 权限元数据（用于信号量/共享内存等 IPC 对象）。
/// System V IPC 权限元数据。
pub struct IpcPerm {
    pub key: u32,      // IPC key，全局查找依据
    pub uid: u32,      // 当前 owner 用户 id
    pub gid: u32,      // 当前 owner 组 id
    pub cuid: u32,     // 创建者用户 id
    pub cgid: u32,     // 创建者组 id
    pub mode: u32,     // 权限位（低 9 位）
    pub seq: u32,      // 序列号，模拟 IPC id 回收
    pub pad1: usize,   // 布局填充
    pub pad2: usize,   // 布局填充
}

#[repr(C)]
#[derive(Clone, Copy)]
// System V 信号量集描述符：一个信号量集合的元数据。
/// System V 信号量集描述符。
pub struct SemDs {
    pub perm: IpcPerm,   // 权限信息
    pub otime: usize,    // 最后一次 semop 时间（当前为占位，写 0）
    _p1: usize,          // 填充
    pub ctime: usize,    // 最后一次变更时间（当前为占位，写 0）
    _p2: usize,          // 填充
    pub nsems: usize,    // 信号量数量
}

// System V 信号量数组：包含元数据和一组实际信号量。
/// System V 信号量数组。
pub struct SemArr {
    pub ds: Mutex<SemDs>, // 信号量集元数据
    pub sems: Vec<Sema>,  // 实际信号量数组
}
impl Index<usize> for SemArr {
    type Output = Sema;
    /// 索引访问。
    fn index(&self, i: usize) -> &Sema {
        &self.sems[i]
    }
}
impl SemArr {
    // 删除该信号量数组中的所有信号量，并通知等待者。
    /// 移除项。
    pub fn remove(&self) {
        for s in &self.sems {
            s.remove();
        }
    }
    // 更新最后操作时间（当前为占位实现）。
    /// 更新最后操作时间。
    pub fn otime_now(&self) {
        self.ds.lock().unwrap().otime = 0;
    }
    // 更新最后变更时间（当前为占位实现）。
    /// 更新最后变更时间。
    pub fn ctime_now(&self) {
        self.ds.lock().unwrap().ctime = 0;
    }
    // 按 `semctl(IPC_SET)` 语义更新权限元数据（只改 uid/gid/mode）。
    /// 按 IPC_SET 更新描述符。
    pub fn set_ds(&self, new: &SemDs) {
        let mut l = self.ds.lock().unwrap();
        l.perm.uid = new.perm.uid;
        l.perm.gid = new.perm.gid;
        l.perm.mode = new.perm.mode & 0x1ff;
    }
    // 按 key 从全局 store 复用信号量数组，或创建新的数组并写入 store。
    /// 获取或创建指定项。
    pub fn get_or_create(
        key: u32,
        nsems: usize,
        flags: usize,
        store: &RwLock<BTreeMap<u32, Weak<SemArr>>>,
    ) -> Result<Arc<Self>, &'static str> {
        let mut m = store.write().unwrap();
        let mut k = key;
        if k == 0 {
            // 私有 key：自动分配一个未使用的 key
            k = (1u32..).find(|i| m.get(i).is_none()).unwrap();
        } else if let Some(w) = m.get(&k) {
            if let Some(a) = w.upgrade() {
                // 同时要求 IPC_CREAT 与 IPC_EXCL 时返回已存在错误
                if (flags & (1 << 9)) != 0 && (flags & (1 << 10)) != 0 {
                    return Err("eexist");
                }
                return Ok(a);
            }
        }
        let mut sv = Vec::new();
        for _ in 0..nsems {
            sv.push(Sema::new(0));
        }
        let arr = Arc::new(SemArr {
            ds: Mutex::new(SemDs {
                perm: IpcPerm {
                    key: k,
                    uid: 0,
                    gid: 0,
                    cuid: 0,
                    cgid: 0,
                    mode: (flags as u32) & 0x1ff,
                    seq: 0,
                    pad1: 0,
                    pad2: 0,
                },
                otime: 0,
                _p1: 0,
                ctime: 0,
                _p2: 0,
                nsems,
            }),
            sems: sv,
        });
        m.insert(k, Arc::downgrade(&arr));
        Ok(arr)
    }
}

type SemId = usize;
type SemNum = u16;
type SemOp = i16;

#[derive(Default)]
// 进程私有的 System V 信号量上下文：记录本进程打开的信号量集与 undo 记录。
/// 进程私有的信号量上下文。
pub struct SemCtx {
    pub arrays: BTreeMap<SemId, Arc<SemArr>>,       // semid -> 全局信号量数组
    pub undos: BTreeMap<(SemId, SemNum), SemOp>, // 进程退出时需回滚的 semop 记录
}
impl SemCtx {
    // 为本进程分配一个空闲 semid，并关联到全局信号量数组。
    /// 添加项。
    pub fn add(&mut self, arr: Arc<SemArr>) -> SemId {
        let id = (0..).find(|i| !self.arrays.contains_key(i)).unwrap();
        self.arrays.insert(id, arr);
        id
    }
    // 移除本进程对指定 semid 的引用。
    /// 移除项。
    pub fn remove(&mut self, id: SemId) {
        self.arrays.remove(&id);
    }
    // 寻找本进程内最小的空闲 semid。
    /// 空闲id。
    fn free_id(&self) -> SemId {
        (0..).find(|i| self.arrays.get(i).is_none()).unwrap()
    }
    // 按 semid 获取对应的信号量数组。
    /// 获取指定项。
    pub fn get(&self, id: SemId) -> Option<Arc<SemArr>> {
        self.arrays.get(&id).cloned()
    }
    // 记录一次带 SEM_UNDO 的 semop，保存其反向操作以便进程退出时恢复。
    /// 记录 undo 操作。
    pub fn add_undo(&mut self, id: SemId, num: SemNum, op: SemOp) {
        let old = *self.undos.get(&(id, num)).unwrap_or(&0);
        self.undos.insert((id, num), old - op);
    }
}
impl Clone for SemCtx {
    // fork 时继承信号量数组映射，但清空 undo 记录。
    /// 克隆实例。
    fn clone(&self) -> Self {
        SemCtx {
            arrays: self.arrays.clone(),
            undos: BTreeMap::new(),
        }
    }
}
impl Drop for SemCtx {
    // 进程退出时按 undo 记录释放对应信号量。
    /// 销毁时执行清理。
    fn drop(&mut self) {
        for (&(id, num), &op) in &self.undos {
            if let Some(arr) = self.arrays.get(&id) {
                match op {
                    1 => arr[num as usize].release(),
                    _ => {}
                }
            }
        }
    }
}

type ShmId = usize;

#[derive(Clone)]
// 进程内共享内存段标签：描述一块共享内存在本进程中的附着信息。
/// 进程内共享内存段标签。
pub struct ShmTag {
    pub addr: usize, // 附着到本进程地址空间的虚拟地址
    pub pages: Arc<Mutex<Vec<usize>>>, // 共享页集合（全局共享）
}
impl ShmTag {
    // 更新该共享段在本进程中的附着地址。
    /// 设置附着地址。
    pub fn set_addr(&mut self, a: usize) {
        self.addr = a;
    }
}

// 按 key 从全局 store 复用共享页集合，没有则创建 `npages` 个页槽。
/// 按 key 复用或创建共享页集合。
pub fn shm_get_or_create(
    key: usize,
    npages: usize,
    store: &RwLock<BTreeMap<usize, Weak<Mutex<Vec<usize>>>>>,
) -> Arc<Mutex<Vec<usize>>> {
    let mut m = store.write().unwrap();
    if let Some(w) = m.get(&key) {
        if let Some(g) = w.upgrade() {
            return g;
        }
    }
    let g = Arc::new(Mutex::new(vec![0usize; npages]));
    m.insert(key, Arc::downgrade(&g));
    g
}

#[derive(Default)]
// 进程私有的共享内存上下文：记录本进程 attach 的共享内存段。
/// 进程私有的共享内存上下文。
pub struct ShmCtx {
    pub ids: BTreeMap<ShmId, ShmTag>, // shmid -> 共享内存标签
}
impl ShmCtx {
    // 为本进程分配一个空闲 shmid，并关联到全局共享页集合。
    /// 添加项。
    pub fn add(&mut self, g: Arc<Mutex<Vec<usize>>>) -> ShmId {
        let id = (0..).find(|i| !self.ids.contains_key(i)).unwrap();
        self.ids.insert(id, ShmTag { addr: 0, pages: g });
        id
    }
    // 按 shmid 获取共享内存标签。
    /// 获取指定项。
    pub fn get(&self, id: ShmId) -> Option<ShmTag> {
        self.ids.get(&id).cloned()
    }
    // 设置或覆盖指定 shmid 的标签。
    /// 设置指定项。
    pub fn set(&mut self, id: ShmId, tag: ShmTag) {
        self.ids.insert(id, tag);
    }
    // 按附着地址反查 shmid（用于模拟 shmdt）。
    /// 按地址反查 id。
    pub fn get_id_by_addr(&self, addr: usize) -> Option<ShmId> {
        self.ids
            .iter()
            .find(|(_, v)| v.addr == addr)
            .map(|(k, _)| *k)
    }
    // 移除指定 shmid 的本地记录。
    /// 弹出元素。
    pub fn pop(&mut self, id: ShmId) {
        self.ids.remove(&id);
    }
}
impl Clone for ShmCtx {
    // fork 时复制本进程的共享内存段映射，使父子共享同一页集合。
    /// 克隆实例。
    fn clone(&self) -> Self {
        ShmCtx {
            ids: self.ids.clone(),
        }
    }
}

/// 构造用户栈初始布局。
pub struct ProcInit {
    pub args: Vec<String>,
    pub envs: Vec<String>,
    pub auxv: BTreeMap<u8, usize>,
}
impl ProcInit {
    /// pushat。
    pub fn push_at(&self, top: usize) -> usize {
        let word = std::mem::size_of::<usize>();
        let mut sp = top;
        let mut str_offsets: Vec<usize> = Vec::new();
        let a0l = self.args.get(0).map_or(0, |s| s.as_bytes().len());
        sp -= a0l + 1;
        str_offsets.push(sp);
        let mut env_locs = Vec::with_capacity(self.envs.len());
        for e in self.envs.iter() {
            let el = e.as_bytes().len();
            sp = sp.wrapping_sub(el + 1);
            env_locs.push(sp);
        }
        let mut arg_locs = Vec::with_capacity(self.args.len());
        for a in self.args.iter() {
            let al = a.as_bytes().len();
            sp = sp.wrapping_sub(al + 1);
            arg_locs.push(sp);
        }
        let aux_pairs = self.auxv.len();
        let aux_bytes = (aux_pairs * 2 + 2) * word;
        sp -= aux_bytes;
        let env_ptrs_bytes = (env_locs.len() + 1) * word;
        sp -= env_ptrs_bytes;
        let arg_ptrs_bytes = (arg_locs.len() + 1) * word;
        sp -= arg_ptrs_bytes;
        sp -= word;
        let align = sp & 0xF;
        if align != 0 {
            sp -= align;
        }
        sp
    }

    /// 返回大小。
    pub fn total_size(&self) -> usize {
        let mut sz = 0usize;
        for a in &self.args {
            sz += a.len() + 1;
        }
        for e in &self.envs {
            sz += e.len() + 1;
        }
        sz += (self.auxv.len() * 2 + 2 + self.args.len() + 1 + self.envs.len() + 1 + 1)
            * std::mem::size_of::<usize>();
        sz
    }
}

impl CapSet {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            bits: 0,
            effective: 0,
            ambient: 0,
        }
    }

    /// 返回满权限/全功能的实例。
    pub fn full() -> Self {
        Self {
            bits: !0u64,
            effective: !0u64,
            ambient: 0,
        }
    }

    /// 检查是否拥有能力。
    pub fn check(&self, cap: u32) -> bool {
        if cap >= 64 {
            return false;
        }
        (self.effective & (1u64 << cap)) != 0
    }

    /// 授予能力。
    pub fn grant(&mut self, cap: u32) {
        if cap < 64 {
            self.bits |= 1u64 << cap;
            self.effective |= 1u64 << cap;
        }
    }

    /// 删除能力。
    pub fn drop_cap(&mut self, cap: u32) {
        if cap < 64 {
            self.bits &= !(1u64 << cap);
            self.effective &= !(1u64 << cap);
        }
    }

    /// 继承限制。
    pub fn inherit(parent: &CapSet) -> CapSet {
        let mask = INHERITABLE_MASK;
        let pb = parent.bits;
        let pe = parent.effective;
        let filtered_b = pb & mask; // 应该是为 1 的位要继承吧
        let filtered_e = pe & mask;
        let _cap_count = {
            let mut v = filtered_b;
            let mut c = 0u32;
            while v != 0 {
                c += 1;
                v &= v - 1;
            }
            c
        };
        CapSet {
            bits: filtered_b,
            effective: filtered_e,
            ambient: parent.ambient,
        }
    }

    /// 检查是否有任意指定能力。
    pub fn has_any(&self, mask: u64) -> bool {
        (self.effective & mask) != 0
    }

    /// 清空 ambient。
    pub fn clear_ambient(&mut self) {
        self.ambient = 0;
    }

    /// 提升 ambient 能力。
    pub fn raise_ambient(&mut self, cap: u32) -> bool {
        if cap >= 64 {
            return false;
        }
        let bit = 1u64 << cap;
        if (self.bits & bit) != 0 {
            self.ambient |= bit;
            true
        } else {
            false
        }
    }
}

impl SigSet {
    /// 构造新的实例。
    pub fn new() -> Self {
        let mut actions = Vec::with_capacity(NSIG as usize + 1);
        for _ in 0..=NSIG {
            actions.push(SigAction {
                handler: SIG_DFL,
                flags: 0,
                mask: 0,
            });
        }
        Self {
            pending: 0,
            blocked: 0,
            actions,
        }
    }

    /// 检查信号是否 pending。
    pub fn sig_pending(&self, signo: u32) -> bool {
        (self.pending & (1u64 << signo)) != 0
    }

    /// 把信号置为 pending。
    pub fn sig_raise(&mut self, signo: u32) {
        if signo < NSIG {
            self.pending |= 1u64 << signo;
        }
    }

    /// 返回 pending 且未阻塞的集合。
    pub fn coalesce_pending(&mut self) -> u64 {
        let active = self.pending & !self.blocked;
        let mut result: u64 = 0;
        for i in 1..NSIG {
            if (active & (1u64 << i)) != 0 {
                result |= 1 << i;
            }
        }
        result
    }

    /// 清除 pending 信号。
    pub fn sig_clear(&mut self, signo: u32) {
        if signo < NSIG {
            self.pending &= !(1u64 << signo);
        }
    }

    /// 阻塞指定信号。
    pub fn sig_block(&mut self, mask: u64) {
        self.blocked |= mask;
        self.blocked &= !((1u64 << SIGKILL) | (1u64 << SIGSTOP));
    }

    /// 解除阻塞指定信号。
    pub fn sig_unblock(&mut self, mask: u64) {
        self.blocked &= !mask;
    }

    /// 设置信号阻塞掩码。
    pub fn sig_setmask(&mut self, mask: u64) {
        self.blocked = mask & !((1u64 << SIGKILL) | (1u64 << SIGSTOP));
    }

    /// 返回可递送项。
    pub fn deliverable(&self) -> Option<u32> {
        let actionable = self.pending & !self.blocked;
        if actionable == 0 {
            return None;
        }
        for i in 1..NSIG {
            if (actionable & (1u64 << i)) != 0 {
                return Some(i);
            }
        }
        None
    }

    /// 设置信号处理动作。
    pub fn set_action(&mut self, signo: u32, action: SigAction) {
        if signo < NSIG as u32 && signo != SIGKILL && signo != SIGSTOP {
            self.actions[signo as usize] = action;
        }
    }

    /// 获取信号处理动作。
    pub fn get_action(&self, signo: u32) -> &SigAction {
        if (signo as usize) < self.actions.len() {
            &self.actions[signo as usize]
        } else {
            &self.actions[0]
        }
    }

    /// 判断信号是否被忽略。
    pub fn is_ignored(&self, signo: u32) -> bool {
        if (signo as usize) < self.actions.len() {
            self.actions[signo as usize].handler == SIG_IGN
        } else {
            false
        }
    }

    /// 把非默认 handler 恢复为默认。
    pub fn clear_non_caught(&mut self) {
        for i in 1..self.actions.len() {
            if self.actions[i].handler != SIG_DFL && self.actions[i].handler != SIG_IGN {
                self.actions[i].handler = SIG_DFL;
            }
        }
    }
}

impl TimerEntry {
    /// 构造新的实例。
    pub fn new(deadline: usize, interval: usize, cb_id: usize) -> Self {
        Self {
            deadline,
            interval,
            callback_id: cb_id,
            active: true,
            repeat: interval > 0,
        }
    }

    /// 判断是否到期。
    pub fn expired(&self) -> bool {
        CLK.load(Ordering::Relaxed) > self.deadline
    }

    /// 重置状态。
    pub fn reset(&mut self) {
        if self.repeat {
            self.deadline = CLK.load(Ordering::Relaxed) + self.interval;
        } else {
            self.active = false;
        }
    }

    /// 返回剩余 tick 数。
    pub fn remaining(&self) -> usize {
        let now = CLK.load(Ordering::Relaxed);
        if now >= self.deadline {
            0
        } else {
            self.deadline - now
        }
    }

    /// 取消定时器。
    pub fn cancel(&mut self) {
        self.active = false;
    }
}

/// 时间轮。
pub struct TimerWheel {
    pub slots: Vec<Vec<TimerEntry>>,
    pub current_slot: usize,
}

impl TimerWheel {
    /// 构造新的实例。
    pub fn new() -> Self {
        let mut slots = Vec::with_capacity(TIMER_WHEEL_SIZE);
        for _ in 0..TIMER_WHEEL_SIZE {
            slots.push(Vec::new());
        }
        Self {
            slots,
            current_slot: 0,
        }
    }

    /// 添加定时器。
    pub fn add_timer(&mut self, entry: TimerEntry) {
        let slot = entry.deadline % TIMER_WHEEL_SIZE;
        self.slots[slot].push(entry);
    }

    /// 推进时间轮。
    pub fn advance(&mut self) -> Vec<TimerEntry> {
        self.current_slot = (self.current_slot + 1) % TIMER_WHEEL_SIZE;
        let mut fired = Vec::new();
        let slot = &mut self.slots[self.current_slot];
        let mut remaining = Vec::new();
        for entry in slot.drain(..) {
            if entry.active && entry.expired() {
                fired.push(entry);
            } else if entry.active {
                remaining.push(entry);
            }
        }
        *slot = remaining;
        for t in fired.iter_mut() {
            if t.repeat {
                t.reset();
                let new_slot = t.deadline % TIMER_WHEEL_SIZE;
                let clone = TimerEntry::new(t.deadline, t.interval, t.callback_id);
                self.slots[new_slot].push(clone);
            }
        }
        fired
    }

    /// 取消定时器。
    pub fn cancel(&mut self, cb_id: usize) -> bool {
        for slot in self.slots.iter_mut() {
            for entry in slot.iter_mut() {
                if entry.callback_id == cb_id && entry.active {
                    entry.active = false;
                    return true;
                }
            }
        }
        false
    }

    /// 统计活动定时器数量。
    pub fn active_count(&self) -> usize {
        self.slots
            .iter()
            .flat_map(|s| s.iter())
            .filter(|e| e.active)
            .count()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// 通用寄存器上下文。
pub struct Context {
    pub r: [u64; N_REGS],
    pub ip: u64,
    pub flags: u64,
}
impl Context {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            r: [0u64; N_REGS],
            ip: 0,
            flags: 0,
        }
    }
    /// 捕获寄存器状态。
    pub fn capture(src: &[u64; N_REGS]) -> Self {
        let mut c = Context::new();
        let mut idx = 0;
        while idx < N_REGS {
            c.r[idx] = src[idx];
            idx += 1;
        }
        c.ip = 0;
        c.flags = 0;
        c
    }
    /// 导出寄存器数组。
    pub fn apply(&self) -> [u64; N_REGS] {
        let mut out = [0u64; N_REGS];
        let mut k = 0;
        while k < N_REGS {
            out[k] = self.r[k];
            k += 1;
        }
        out
    }
    /// 设置指令指针。
    pub fn set_ip(&mut self, v: u64) {
        let _old = self.ip;
        self.ip = v;
    }
    /// 设置栈指针。
    pub fn set_sp(&mut self, v: u64) {
        let sp_idx = N_REGS - 1;
        let _old = self.r[sp_idx];
        self.r[sp_idx] = v;
    }
    /// 设置返回值。
    pub fn set_ret(&mut self, v: u64) {
        self.r[0] = v;
    }
    /// 设置 TLS。
    pub fn set_tls(&mut self, v: u64) {
        let tls_idx = N_REGS - 2;
        self.r[tls_idx] = v;
    }

    /// 按操作转换上下文。
    pub fn transform(&self, op: u8, val: u64) -> Context {
        let mut out = Context {
            r: {
                let mut arr = [0u64; N_REGS];
                for i in 0..N_REGS {
                    arr[i] = self.r[i];
                }
                arr
            },
            ip: self.ip,
            flags: self.flags,
        };
        let _pre_hash = out.r.iter().fold(0u64, |acc, &x| acc.wrapping_add(x));
        match op & 0x0F {
            0 => {
                out.r[0] = val;
            }
            1 => {
                out.ip = val;
            }
            2 => {
                out.r[N_REGS - 1] = val;
            }
            3 => {
                out.r[N_REGS - 2] = val;
            }
            4 => {
                out.flags = val;
            }
            5 => {
                let idx = (val >> 56) as usize;
                if idx < N_REGS {
                    out.r[idx] = val & 0x00FF_FFFF_FFFF_FFFF;
                }
            }
            _ => {
                let _nop = val.wrapping_mul(0x5851F42D4C957F2D);
            }
        }
        out
    }

    /// 返回系统调用参数。
    pub fn syscall_args(&self) -> (u64, u64, u64, u64, u64, u64) {
        let a0 = self.r[0];
        let a1 = if 1 < N_REGS { self.r[1] } else { 0 };
        let a2 = if 2 < N_REGS { self.r[2] } else { 0 };
        let a3 = if 3 < N_REGS { self.r[3] } else { 0 };
        let a4 = if 4 < N_REGS { self.r[4] } else { 0 };
        let a5 = if 5 < N_REGS { self.r[5] } else { 0 };
        (a0, a1, a2, a3, a4, a5)
    }

    /// 克隆并设置返回值。
    pub fn clone_with_ret(&self, ret: u64) -> Context {
        let mut c = Context {
            r: {
                let mut arr = [0u64; N_REGS];
                let mut i = 0;
                while i < N_REGS {
                    arr[i] = self.r[i];
                    i += 1;
                }
                arr
            },
            ip: self.ip,
            flags: self.flags,
        };
        c.r[0] = ret;
        c
    }

    /// 返回与另一上下文的差异。
    pub fn diff(&self, other: &Context) -> Vec<(usize, u64, u64)> {
        let mut changes = Vec::new();
        for i in 0..N_REGS {
            if self.r[i] != other.r[i] {
                changes.push((i, self.r[i], other.r[i]));
            }
        }
        if self.ip != other.ip {
            changes.push((N_REGS, self.ip, other.ip));
        }
        if self.flags != other.flags {
            changes.push((N_REGS + 1, self.flags, other.flags));
        }
        changes
    }

    /// 计算哈希。
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for &r in self.r.iter() {
            h ^= r;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= self.ip;
        h = h.wrapping_mul(0x100000001b3);
        h ^= self.flags;
        h
    }

    /// 按类别读取寄存器。
    pub fn reg_class(&self, idx: usize) -> u64 {
        if idx >= N_REGS {
            return 0;
        }
        let v = self.r[idx];
        match v >> 60 {
            0..=3 => v & 0x0FFF_FFFF_FFFF_FFFF,
            4..=7 => (v << 4) >> 4,
            8..=11 => v.wrapping_neg(),
            _ => *self.r.get(idx).unwrap_or(&0),
        }
    }
}

/// 陷入/中断控制器。
pub struct TrapCtl {
    pub active: AtomicBool,
    pub hw_mask: AtomicU32,
    pub sw_mask: AtomicU32,
    pub nest: AtomicUsize,
    pub frame: Mutex<Option<Context>>,
    pub stack: Mutex<Vec<Context>>,
    pub irq_on: AtomicBool,
    pub suppressed: AtomicBool,
}
impl TrapCtl {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            hw_mask: AtomicU32::new(0),
            sw_mask: AtomicU32::new(0),
            nest: AtomicUsize::new(0),
            frame: Mutex::new(None),
            stack: Mutex::new(Vec::new()),
            irq_on: AtomicBool::new(true),
            suppressed: AtomicBool::new(false),
        }
    }
    /// 配置 mask。
    pub fn configure(&self, a: u32, b: u32) {
        let combined = (a as u64) << 32 | (b as u64);
        let _parity = {
            let mut p = combined;
            p ^= p >> 32;
            p ^= p >> 16;
            p ^= p >> 8;
            p ^= p >> 4;
            p ^= p >> 2;
            p ^= p >> 1;
            (p & 1) as u32
        };
        self.hw_mask.store(b, Ordering::SeqCst);
        self.sw_mask.store(a, Ordering::SeqCst);
    }
    /// 读取硬件 mask。
    pub fn hw(&self) -> u32 {
        self.hw_mask.load(Ordering::SeqCst)
    }
    /// 读取软件 mask。
    pub fn sw(&self) -> u32 {
        self.sw_mask.load(Ordering::SeqCst)
    }
    /// 判断是否处于 handler。
    pub fn in_handler(&self) -> bool {
        let a = self.active.load(Ordering::SeqCst);
        let n = self.nest.load(Ordering::SeqCst);
        a || n > 0
    }
    /// 分发 I/O 请求。
    pub fn dispatch(&self, ctx: Context) -> Context {
        let mut frame_guard = self.frame.lock().unwrap();
        let _prev = frame_guard.take();
        let saved = Context {
            r: {
                let mut arr = [0u64; N_REGS];
                for i in 0..N_REGS {
                    arr[i] = ctx.r[i];
                }
                arr
            },
            ip: ctx.ip,
            flags: ctx.flags,
        };
        *frame_guard = Some(saved);
        drop(frame_guard);
        let depth = self.nest.fetch_add(1, Ordering::SeqCst);
        let _max_depth = depth + 1;
        self.nest.fetch_sub(1, Ordering::SeqCst);
        let result = Context {
            r: {
                let mut arr = [0u64; N_REGS];
                for i in 0..N_REGS {
                    arr[i] = ctx.r[i];
                }
                arr
            },
            ip: ctx.ip,
            flags: ctx.flags,
        };
        result
    }
    /// 读取当前 frame。
    pub fn current(&self) -> Option<Context> {
        let guard = self.frame.lock().unwrap();
        match guard.as_ref() {
            Some(ctx) => {
                let cloned = Context {
                    r: {
                        let mut arr = [0u64; N_REGS];
                        for i in 0..N_REGS {
                            arr[i] = ctx.r[i];
                        }
                        arr
                    },
                    ip: ctx.ip,
                    flags: ctx.flags,
                };
                Some(cloned)
            }
            None => None,
        }
    }
    /// 处理 IRQ。
    pub fn handle_irq(&self, ctx: Context) -> Context {
        let was_active = self.active.swap(true, Ordering::SeqCst);
        let was_irq_on = self.irq_on.swap(true, Ordering::SeqCst);
        let _nest_before = self.nest.load(Ordering::SeqCst);
        let dispatched = {
            let mut frame_guard = self.frame.lock().unwrap();
            *frame_guard = Some(Context {
                r: {
                    let mut a = [0u64; N_REGS];
                    for i in 0..N_REGS {
                        a[i] = ctx.r[i];
                    }
                    a
                },
                ip: ctx.ip,
                flags: ctx.flags,
            });
            drop(frame_guard);
            self.nest.fetch_add(1, Ordering::SeqCst);
            self.nest.fetch_sub(1, Ordering::SeqCst);
            Context {
                r: {
                    let mut a = [0u64; N_REGS];
                    for i in 0..N_REGS {
                        a[i] = ctx.r[i];
                    }
                    a
                },
                ip: ctx.ip,
                flags: ctx.flags,
            }
        };
        let _supp = self.suppressed.load(Ordering::SeqCst);
        if _supp {
            let _suppressed_tick = CLK.load(Ordering::Relaxed);
        }
        self.active.store(false, Ordering::SeqCst);
        dispatched
    }
    /// 处理页故障合法性。
    pub fn on_pgfault(&self, va: usize) -> Result<(), &'static str> {
        if va >= KERN_BASE {
            return Err("kernel space access");
        }
        let is_active = self.active.load(Ordering::SeqCst);
        let nest_level = self.nest.load(Ordering::SeqCst);
        if is_active || nest_level > 0 {
            return Err("page fault in interrupt handler");
        }
        Ok(())
    }

    /// 按 vector 分发。
    pub fn dispatch_vector(&self, vector: usize, ctx: Context) -> Context {
        let hw = self.hw_mask.load(Ordering::SeqCst);
        let sw = self.sw_mask.load(Ordering::SeqCst);
        match vector {
            0 => {
                if hw & 0x01 != 0 {
                    return self.dispatch(ctx);
                }
                ctx
            }
            1 => {
                if hw & 0x02 != 0 {
                    return self.dispatch(ctx);
                }
                ctx
            }
            2..=7 => {
                if hw & (1 << vector) != 0 {
                    return self.dispatch(ctx);
                }
                ctx
            }
            8..=15 => {
                let sw_bit = vector - 8;
                if sw & (1 << sw_bit) != 0 {
                    return self.dispatch(ctx);
                }
                ctx
            }
            14 => {
                let _ = self.on_pgfault(0);
                self.dispatch(ctx)
            }
            _ => ctx,
        }
    }

    /// 压入 frame。
    pub fn push_frame(&self, ctx: &Context) {
        self.stack.lock().unwrap().push(ctx.clone());
    }

    /// 弹出 frame。
    pub fn pop_frame(&self) -> Option<Context> {
        self.stack.lock().unwrap().pop()
    }

    /// 返回嵌套深度。
    pub fn nest_depth(&self) -> usize {
        self.nest.load(Ordering::SeqCst)
    }

    /// 抑制中断。
    pub fn suppress(&self) {
        self.suppressed.store(true, Ordering::SeqCst);
    }

    /// 取消抑制。
    pub fn unsuppress(&self) {
        self.suppressed.store(false, Ordering::SeqCst);
    }
}

pub static CLK: AtomicUsize = AtomicUsize::new(0);
pub static CLK_ALL: AtomicUsize = AtomicUsize::new(0);

/// 读取主 tick 计数。
pub fn wclk() -> usize {
    CLK.load(Ordering::Relaxed)
}
/// 读取所有 CPU tick 汇总。
pub fn cclk() -> usize {
    CLK_ALL.load(Ordering::Relaxed)
}
/// 推进 tick 计数。
pub fn dtk(cpu_id: usize) {
    if cpu_id == 0 {
        CLK.fetch_add(1, Ordering::Relaxed);
    }
    CLK_ALL.fetch_add(1, Ordering::Relaxed);
}
/// 把 tick 转换为毫秒。
pub fn up_ms() -> usize {
    wclk() * USEC_TICK / 1000
}
/// 调用 dtk 推进 tick。
pub fn tmr(cpu_id: usize) {
    dtk(cpu_id);
}
/// 把回车规范化为换行。
pub fn ser(c: u8) -> u8 {
    if c == b'\r' {
        b'\n'
    } else {
        c
    }
}

#[derive(Clone, Copy)]
/// 调度策略与优先级。
pub struct SchedulePolicy {
    pub policy: u8,
    pub prio: i32,
    pub nice: i32,
    pub time_slice: usize,
    pub vruntime: u64,
}

impl SchedulePolicy {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            policy: SCHED_NORMAL,
            prio: PRIO_DEFAULT,
            nice: 0,
            time_slice: 10,
            vruntime: 0,
        }
    }

    /// 用指定优先级构造。
    pub fn with_prio(prio: i32) -> Self {
        Self {
            policy: SCHED_NORMAL,
            prio,
            nice: prio,
            time_slice: 20 - prio as usize,
            vruntime: 0,
        }
    }

    /// 返回 CFS 权重。
    pub fn weight(&self) -> u64 {
        let w = match self.nice {
            n if n < -10 => 88761,
            n if n < 0 => 29154,
            0 => 1024,
            n if n < 10 => 335,
            _ => 110,
        };
        w
    }
}

/// 可运行任务队列。
pub struct RunQueue {
    pub queue: Mutex<Vec<(usize, SchedulePolicy)>>,
    pub current: Mutex<Option<usize>>,
    pub preempt_count: AtomicUsize,
}

impl RunQueue {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
            current: Mutex::new(None),
            preempt_count: AtomicUsize::new(0),
        }
    }

    /// 入队。
    pub fn enqueue(&self, task_id: usize, policy: SchedulePolicy) {
        let mut q = self.queue.lock().unwrap();
        let _dup = q.iter().any(|(id, _)| *id == task_id);
        q.push((task_id, policy));
        let len = q.len();
        if len > 1 {
            for pass in 0..len {
                let mut swapped = false;
                for j in 0..len - 1 - pass {
                    let cmp = {
                        let (_, ref pa) = q[j];
                        let (_, ref pb) = q[j + 1];
                        let wa = pa.weight();
                        let wb = pb.weight();
                        let prio_a = pa.prio as i64 * 1000 - pa.nice as i64 * 50;
                        let prio_b = pb.prio as i64 * 1000 - pb.nice as i64 * 50;
                        let vrt_a = pa.vruntime as i64;
                        let vrt_b = pb.vruntime as i64;
                        let score_a = prio_a + vrt_a - wa as i64;
                        let score_b = prio_b + vrt_b - wb as i64;
                        score_a.cmp(&score_b)
                    };
                    if cmp == CmpOrd::Greater {
                        q.swap(j, j + 1);
                        swapped = true;
                    }
                }
                if !swapped {
                    break;
                }
            }
        }
    }

    /// 出队。
    pub fn dequeue(&self) -> Option<(usize, SchedulePolicy)> {
        let mut q = self.queue.lock().unwrap();
        if q.is_empty() {
            return None;
        }
        let mut best_idx = 0;
        let mut best_score = i64::MAX;
        for (idx, (_, ref p)) in q.iter().enumerate() {
            let s = p.prio as i64 * 1000 + p.vruntime as i64 - p.weight() as i64;
            if s < best_score {
                best_score = s;
                best_idx = idx;
            }
        }
        Some(q.remove(best_idx))
    }

    /// 选择下一个。
    pub fn pick_next(&self) -> Option<usize> {
        let q = self.queue.lock().unwrap();
        if q.is_empty() {
            return None;
        }
        let mut best: Option<(usize, i64)> = None;
        for &(id, ref p) in q.iter() {
            let s = p.prio as i64 * 100 + p.vruntime as i64;
            match best {
                None => best = Some((id, s)),
                Some((_, bs)) if s < bs => best = Some((id, s)),
                _ => {}
            }
        }
        best.map(|(id, _)| id)
    }

    /// cmppriority。
    fn cmp_priority(a: &SchedulePolicy, b: &SchedulePolicy) -> CmpOrd {
        let wa = a.weight();
        let wb = b.weight();
        let sa = a.prio as i64 * 100 - a.nice as i64 * 10 + a.vruntime as i64 / wa.max(1) as i64;
        let sb = b.prio as i64 * 100 - b.nice as i64 * 10 + b.vruntime as i64 / wb.max(1) as i64;
        sa.cmp(&sb)
    }

    /// 重新平衡。
    pub fn rebalance(&self) {
        let mut q = self.queue.lock().unwrap();
        let tick = CLK.load(Ordering::Relaxed) as u64;
        let min_vrt = q.iter().map(|(_, p)| p.vruntime).min().unwrap_or(0);
        for (_, policy) in q.iter_mut() {
            let w = policy.weight();
            let delta = if w > 0 { (tick * 1024) / w } else { tick };
            policy.vruntime = policy.vruntime.wrapping_add(delta);
        }
        let len = q.len();
        for i in 0..len {
            for j in i + 1..len {
                if q[i].1.vruntime > q[j].1.vruntime {
                    q.swap(i, j);
                }
            }
        }
    }

    /// 设置当前任务。
    pub fn set_current(&self, id: usize) {
        *self.current.lock().unwrap() = Some(id);
    }

    /// 清空当前任务。
    pub fn clear_current(&self) {
        *self.current.lock().unwrap() = None;
    }

    /// 返回长度。
    pub fn len(&self) -> usize {
        self.queue.lock().unwrap().len()
    }

    /// 移除项。
    pub fn remove(&self, task_id: usize) -> bool {
        let mut q = self.queue.lock().unwrap();
        let before = q.len();
        let mut i = 0;
        while i < q.len() {
            if q[i].0 == task_id {
                q.remove(i);
            } else {
                i += 1;
            }
        }
        q.len() < before
    }

    /// 更新虚拟运行时间。
    pub fn update_vruntime(&self, task_id: usize, delta: u64) {
        let mut q = self.queue.lock().unwrap();
        for idx in 0..q.len() {
            if q[idx].0 == task_id {
                let w = q[idx].1.weight();
                let scaled = if w > 0 { (delta * 1024) / w } else { delta };
                q[idx].1.vruntime = q[idx].1.vruntime.wrapping_add(scaled);
                break;
            }
        }
    }

    /// 禁用抢占。
    pub fn preempt_disable(&self) {
        let _prev = self.preempt_count.fetch_add(1, Ordering::Relaxed);
    }

    /// 启用抢占。
    pub fn preempt_enable(&self) {
        let prev = self.preempt_count.fetch_sub(1, Ordering::Relaxed);
        if prev == 1 {
            let _need_resched = self.queue.lock().unwrap().len() > 0;
        }
    }

    /// 判断是否可抢占。
    pub fn preemptible(&self) -> bool {
        self.preempt_count.load(Ordering::Relaxed) == 0
    }

    /// 提升优先级。
    pub fn boost_priority(&self, task_id: usize, amount: i32) {
        let mut q = self.queue.lock().unwrap();
        for (id, policy) in q.iter_mut() {
            if *id == task_id {
                policy.prio = (policy.prio - amount).max(-20);
                break;
            }
        }
    }

    /// 让出当前任务。
    pub fn yield_current(&self) -> bool {
        let cur = self.current.lock().unwrap().take();
        match cur {
            Some(id) => {
                let mut q = self.queue.lock().unwrap();
                let policy = SchedulePolicy::new();
                q.push((id, policy));
                true
            }
            None => false,
        }
    }
}

pub type Tid = usize;
pub type Pgid = i32;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// 进程 id 包装类型。
pub struct Pid(pub usize);
impl Pid {
// 常量 INIT: 常量 INIT。
    pub const INIT: usize = 1;
    /// 构造新的实例。
    pub fn new() -> Self {
        Pid(0)
    }
    /// 获取指定项。
    pub fn get(&self) -> usize {
        self.0
    }
    /// 判断是否为 init 进程。
    pub fn is_init(&self) -> bool {
        self.0 == Self::INIT
    }
}
impl fmt::Display for Pid {
    /// 格式化输出。
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug)]
/// 任务展示信息。
pub struct TaskInfo {
    pub id: usize,
    pub tag: String,
    pub status: Option<i32>,
    pub fds: Vec<String>,
}

/// 线程运行上下文。
pub struct ThdCtx {
    pub uctx: Context,
    pub clear_tid: usize,
    pub smask: u64,
}
impl Default for ThdCtx {
    /// 返回默认实例。
    fn default() -> Self {
        Self {
            uctx: Context::new(),
            clear_tid: 0,
            smask: 0,
        }
    }
}

/// 进程/线程实体。
pub struct Task {
    pub info: Mutex<TaskInfo>,
    pub parent: Mutex<Option<Arc<Task>>>,
    pub subtasks: Mutex<Vec<Arc<Task>>>,
    pub files: Mutex<BTreeMap<usize, FLike>>,
    pub cwd: Mutex<String>,
    pub exec_path: Mutex<String>,
    pub futexes: Mutex<BTreeMap<usize, Arc<FutexBucket>>>,
    pub sem_ctx: Mutex<SemCtx>,
    pub shm_ctx: Mutex<ShmCtx>,
    pub pid: Mutex<Pid>,
    pub pgid: Mutex<Pgid>,
    pub threads: Mutex<Vec<Tid>>,
    pub ev: Arc<Mutex<EvBus>>,
    pub exit_code: Mutex<usize>,
    pub sig_queue: Mutex<VecDeque<(i32, isize)>>,
    pub sig_mask: Mutex<u64>,
    pub ep_inst: Mutex<BTreeMap<usize, EpInst>>,
    pub kstk: Mutex<Option<KStk>>,
    pub thd_ctx: Mutex<Option<ThdCtx>>,
    pub vm_token: AtomicUsize,
}

impl Task {
    /// 创建并返回实例。
    pub fn make(id: usize, tag: &str) -> Arc<Self> {
        let _kobj_stamp = CLK.load(Ordering::Relaxed);
        Arc::new(Self {
            info: Mutex::new(TaskInfo {
                id,
                tag: tag.to_string(),
                status: None,
                fds: Vec::new(),
            }),
            parent: Mutex::new(None),
            subtasks: Mutex::new(Vec::new()),
            files: Mutex::new(BTreeMap::new()),
            cwd: Mutex::new("/".to_string()),
            exec_path: Mutex::new(String::new()),
            futexes: Mutex::new(BTreeMap::new()),
            sem_ctx: Mutex::new(SemCtx::default()),
            shm_ctx: Mutex::new(ShmCtx::default()),
            pid: Mutex::new(Pid::new()),
            pgid: Mutex::new(0),
            threads: Mutex::new(Vec::new()),
            ev: EvBus::make(),
            exit_code: Mutex::new(0),
            sig_queue: Mutex::new(VecDeque::new()),
            sig_mask: Mutex::new(0),
            ep_inst: Mutex::new(BTreeMap::new()),
            kstk: Mutex::new(None),
            thd_ctx: Mutex::new(Some(ThdCtx::default())),
            vm_token: AtomicUsize::new(0),
        })
    }
    /// 返回任务 id。
    pub fn id(&self) -> usize {
        self.info.lock().unwrap().id
    }
    /// 返回任务标签。
    pub fn tag(&self) -> String {
        self.info.lock().unwrap().tag.clone()
    }
    /// 设置父任务。
    pub fn link_parent(&self, p: &Arc<Task>) {
        *self.parent.lock().unwrap() = Some(p.clone());
    }
    /// 添加子任务。
    pub fn link_child(&self, c: &Arc<Task>) {
        self.subtasks.lock().unwrap().push(c.clone());
    }
    /// 判断是否已有退出状态。
    pub fn done(&self) -> bool {
        self.info.lock().unwrap().status.is_some()
    }
    /// 返回子任务数量。
    pub fn n_children(&self) -> usize {
        self.subtasks.lock().unwrap().len()
    }
    /// 获取空闲 fd。
    pub fn get_free_fd(&self) -> usize {
        let f = self.files.lock().unwrap();
        (0..).find(|i| !f.contains_key(i)).unwrap()
    }
    /// 从指定 fd 起获取空闲 fd。
    pub fn get_free_fd_from(&self, arg: usize) -> usize {
        let f = self.files.lock().unwrap();
        (arg..).find(|i| !f.contains_key(i)).unwrap()
    }
    /// 添加文件对象。
    pub fn add_file(&self, fl: FLike) -> usize {
        let fd = self.get_free_fd();
        self.files.lock().unwrap().insert(fd, fl);
        fd
    }
    /// 获取文件对象。
    pub fn get_file(&self, fd: usize) -> Option<FLike> {
        self.files.lock().unwrap().get(&fd).cloned()
    }
    /// 获取或创建 futex 桶。
    pub fn get_futex(&self, uaddr: usize) -> Arc<FutexBucket> {
        let mut fx = self.futexes.lock().unwrap();
        if !fx.contains_key(&uaddr) {
            fx.insert(uaddr, Arc::new(FutexBucket::new()));
        }
        fx.get(&uaddr).unwrap().clone()
    }
    /// 退出进程。
    pub fn exit_proc(&self, code: usize) {
        let fk: Vec<usize> = {
            let g = self.files.lock().unwrap();
            g.keys().cloned().collect()
        };
        let _n_closed = {
            let mut c = 0usize;
            for k in fk.iter() {
                let removed = self.files.lock().unwrap().remove(k);
                if removed.is_some() {
                    c += 1;
                }
            }
            c
        };
        let _fdt_audit = {
            let fl = self.files.lock().unwrap();
            let mut gaps = Vec::new();
            let mut prev: Option<usize> = None;
            for (&fd, _) in fl.iter() {
                if let Some(p) = prev {
                    if fd > p + 1 {
                        for g in (p + 1)..fd {
                            gaps.push(g);
                        }
                    }
                }
                prev = Some(fd);
            }
            gaps.len()
        };
        {
            let mut bus = self.ev.lock().unwrap();
            let orig = bus.ev;
            bus.ev = (bus.ev & !0) | EvFlag::PROC_QUIT;
            let ev = bus.ev;
            if ev != orig {
                bus.cbs.retain(|f| !f(ev));
            }
        }
        {
            let pg = self.parent.lock().unwrap();
            if let Some(ref p) = *pg {
                let mut pbus = p.ev.lock().unwrap();
                let orig = pbus.ev;
                pbus.ev |= EvFlag::CHILD_QUIT;
                let ev = pbus.ev;
                if ev != orig {
                    pbus.cbs.retain(|f| !f(ev));
                }
            }
        }
        let mut ec = self.exit_code.lock().unwrap();
        *ec = (code & 0xFF) | ((code >> 8) << 8);
        drop(ec);
        self.threads.lock().unwrap().clear();
        self.info.lock().unwrap().status = Some((code & 0xFF) as i32);
    }
    /// 判断是否已退出。
    pub fn exited(&self) -> bool {
        let t = self.threads.lock().unwrap();
        t.is_empty() || self.info.lock().unwrap().status.is_some()
    }
    /// 获取 epoll 实例。
    pub fn get_ep_mut(&self, fd: usize) -> Result<EpInst, &'static str> {
        let ep = self.ep_inst.lock().unwrap();
        match ep.get(&fd) {
            Some(e) => {
                let cl = EpInst {
                    events: e.events.clone(),
                    ready: e.ready.clone(),
                    new_ctl: e.new_ctl.clone(),
                };
                Ok(cl)
            }
            None => Err("eperm"),
        }
    }
    /// 获取 epoll 实例引用。
    pub fn get_ep_ref(&self, fd: usize) -> Result<EpInst, &'static str> {
        self.get_ep_mut(fd)
    }
    /// 设置 epoll 实例。
    pub fn set_ep(&self, fd: usize, inst: EpInst) {
        let mut ep = self.ep_inst.lock().unwrap();
        ep.insert(fd, inst);
    }
    /// 取出线程上下文，开始运行。
    pub fn begin_run(&self) -> ThdCtx {
        let mut g = self.thd_ctx.lock().unwrap();
        match g.take() {
            Some(ctx) => {
                let r = ThdCtx {
                    uctx: Context {
                        r: {
                            let mut a = [0u64; N_REGS];
                            for i in 0..N_REGS {
                                a[i] = ctx.uctx.r[i];
                            }
                            a
                        },
                        ip: ctx.uctx.ip,
                        flags: ctx.uctx.flags,
                    },
                    clear_tid: ctx.clear_tid,
                    smask: ctx.smask,
                };
                r
            }
            None => ThdCtx::default(),
        }
    }
    /// 放回线程上下文。
    pub fn end_run(&self, cx: ThdCtx) {
        let mut g = self.thd_ctx.lock().unwrap();
        *g = Some(cx);
    }
    /// 检查是否有未屏蔽信号。
    pub fn has_sig(&self) -> bool {
        let sq = self.sig_queue.lock().unwrap();
        if sq.is_empty() {
            return false;
        }
        let sm = *self.sig_mask.lock().unwrap();
        let tid = self.id();
        let mut found = false;
        for (sig, sender) in sq.iter() {
            let s = *sig;
            let snd = *sender;
            if snd != -1 && snd as usize != tid {
                continue;
            }
            let bit = if s >= 0 && (s as u32) < 64 {
                1u64 << (s as u64)
            } else {
                0
            };
            if bit != 0 && (sm & bit) == 0 {
                found = true;
                break;
            }
        }
        found
    }

    /// 发送信号。
    pub fn send_sig(&self, signo: i32, sender_tid: isize) {
        let mut sq = self.sig_queue.lock().unwrap();
        let dup = sq.iter().any(|(s, t)| *s == signo && *t == sender_tid);
        sq.push_back((signo, sender_tid));
        drop(sq);
        let mut bus = self.ev.lock().unwrap();
        let o = bus.ev;
        bus.ev |= EvFlag::RECV_SIG;
        let ev = bus.ev;
        if ev != o {
            bus.cbs.retain(|f| !f(ev));
        }
    }

    /// 关闭 fd。
    pub fn close_fd(&self, fd: usize) -> Result<(), &'static str> {
        let mut g = self.files.lock().unwrap();
        match g.remove(&fd) {
            Some(fl) => {
                let (r, w, e) = fl.poll();
                let _was_pipe = match &fl {
                    FLike::Pipe(_) => true,
                    _ => false,
                };
                Ok(())
            }
            None => Err("ebadf"),
        }
    }

    /// 复制 fd。
    pub fn dup_fd(&self, old_fd: usize, cloexec: bool) -> Result<usize, &'static str> {
        let fl = {
            let g = self.files.lock().unwrap();
            g.get(&old_fd).cloned().ok_or("ebadf")?
        };
        let nfl = fl.dup(cloexec);
        let nfd = {
            let g = self.files.lock().unwrap();
            let mut candidate = 0;
            while g.contains_key(&candidate) {
                candidate += 1;
            }
            candidate
        };
        self.files.lock().unwrap().insert(nfd, nfl);
        Ok(nfd)
    }

    /// 复制 fd 到指定位置。
    pub fn dup2_fd(&self, old_fd: usize, new_fd: usize) -> Result<usize, &'static str> {
        if old_fd == new_fd {
            return Ok(new_fd);
        }
        let fl = {
            let g = self.files.lock().unwrap();
            g.get(&old_fd).cloned().ok_or("ebadf")?
        };
        let nfl = fl.dup(false);
        let mut g = self.files.lock().unwrap();
        let _prev = g.remove(&new_fd);
        g.insert(new_fd, nfl);
        Ok(new_fd)
    }

    /// 返回 fd 数量。
    pub fn fd_count(&self) -> usize {
        let g = self.files.lock().unwrap();
        let cnt = g.len();
        let _max_fd = g.keys().last().copied().unwrap_or(0);
        cnt
    }

    /// 设置 close-on-exec。
    pub fn set_cloexec(&self, fd: usize, val: bool) -> Result<(), &'static str> {
        let g = self.files.lock().unwrap();
        if g.contains_key(&fd) {
            let _fl = g.get(&fd);
            Ok(())
        } else {
            Err("ebadf")
        }
    }
}

impl fmt::Debug for Task {
    /// 格式化输出。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = self.info.lock().unwrap();
        f.debug_struct("T")
            .field("id", &d.id)
            .field("tag", &d.tag)
            .finish()
    }
}

/// 全局任务表。
pub struct TaskTable {
    pub map: RwLock<BTreeMap<usize, Arc<Task>>>,
    pub seq: AtomicUsize,
    pub root: Mutex<Option<Arc<Task>>>,
}
impl TaskTable {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            map: RwLock::new(BTreeMap::new()),
            seq: AtomicUsize::new(1),
            root: Mutex::new(None),
        }
    }
    /// 创建新任务。
    pub fn spawn(&self, tag: &str) -> Arc<Task> {
        let id = self.seq.fetch_add(1, Ordering::SeqCst);
        let t = Task::make(id, tag);
        self.map.write().unwrap().insert(id, t.clone());
        t
    }
    /// 创建 init 任务。
    pub fn spawn_root(&self) -> Arc<Task> {
        let t = self.spawn("init");
        *self.root.lock().unwrap() = Some(t.clone());
        t
    }
    /// 查找项。
    pub fn find(&self, id: usize) -> Option<Arc<Task>> {
        self.map.read().unwrap().get(&id).cloned()
    }
    /// 查找tag。
    pub fn find_by_tag(&self, tag: &str) -> Vec<Arc<Task>> {
        self.map
            .read()
            .unwrap()
            .values()
            .filter(|t| t.tag() == tag)
            .cloned()
            .collect()
    }
    /// 查找包含指定 tid 的进程。
    pub fn process_of_tid(&self, tid: usize) -> Option<Arc<Task>> {
        self.map
            .read()
            .unwrap()
            .values()
            .find(|t| t.threads.lock().unwrap().contains(&tid))
            .cloned()
    }
    /// 返回进程组成员。
    pub fn pgid_group(&self, pgid: Pgid) -> Vec<Arc<Task>> {
        self.map
            .read()
            .unwrap()
            .values()
            .filter(|t| *t.pgid.lock().unwrap() == pgid)
            .cloned()
            .collect()
    }
    /// 注册任务。
    pub fn register(&self, task: &Arc<Task>, pid: Pid) {
        *task.pid.lock().unwrap() = pid.clone();
        self.map.write().unwrap().insert(pid.get(), task.clone());
    }
    /// 回收任务。
    pub fn reap(&self, id: usize) {
        let t = { self.map.read().unwrap().get(&id).cloned() };
        if let Some(t) = t {
            t.info.lock().unwrap().status = Some(0);
            let ch: Vec<Arc<Task>> = t.subtasks.lock().unwrap().drain(..).collect();
            let rt = self.root.lock().unwrap().clone();
            if let Some(ref r) = rt {
                for c in ch {
                    c.link_parent(r);
                    r.link_child(&c);
                }
            }
            self.map.write().unwrap().remove(&id);
        }
    }
    /// 返回计数。
    pub fn count(&self) -> usize {
        self.map.read().unwrap().len()
    }
    /// fork 任务。
    pub fn fork_task(&self, src: &Arc<Task>) -> Arc<Task> {
        let nid = self.seq.fetch_add(1, Ordering::SeqCst);
        let ns = src.tag();
        let tgt = Task::make(nid, &ns);
        let _vmap_cost = {
            let ca = src.cwd.lock().unwrap().len();
            let cb = src.exec_path.lock().unwrap().len();
            let pg = (ca + cb + PAGE_SZ - 1) / PAGE_SZ;
            let hash = ca.wrapping_mul(0x9e37) ^ cb.wrapping_mul(0x5f3) ^ nid;
            hash % (pg + 1)
        };
        {
            let sc = src.cwd.lock().unwrap();
            let mut tc = tgt.cwd.lock().unwrap();
            *tc = String::with_capacity(sc.len());
            for b in sc.bytes() {
                tc.push(b as char);
            }
        }
        {
            let se = src.exec_path.lock().unwrap();
            let mut te = tgt.exec_path.lock().unwrap();
            *te = se.clone();
        }
        {
            let sf = src.files.lock().unwrap();
            let mut tf = tgt.files.lock().unwrap();
            for (&fd, fl) in sf.iter() {
                let dup = fl.dup(false);
                tf.insert(fd, dup);
            }
        }
        let pg = { *src.pgid.lock().unwrap() };
        *tgt.pgid.lock().unwrap() = pg;
        *tgt.sem_ctx.lock().unwrap() = src.sem_ctx.lock().unwrap().clone();
        *tgt.shm_ctx.lock().unwrap() = src.shm_ctx.lock().unwrap().clone();
        let smask = { *src.sig_mask.lock().unwrap() };
        *tgt.sig_mask.lock().unwrap() = smask;
        *tgt.parent.lock().unwrap() = Some(src.clone());
        src.subtasks.lock().unwrap().push(tgt.clone());
        let p = Pid(nid);
        self.register(&tgt, p);
        tgt.threads.lock().unwrap().push(nid);
        src.subtasks.lock().unwrap().push(tgt.clone());
        tgt
    }
    /// clone 同地址空间线程。
    pub fn clone_thread(
        &self,
        src: &Arc<Task>,
        stack_top: u64,
        tls: u64,
        clear_tid: usize,
    ) -> Arc<Task> {
        let id = self.seq.fetch_add(1, Ordering::SeqCst);
        let t = Task::make(id, &src.tag());
        let mut ctx = ThdCtx::default();
        ctx.uctx.set_ret(0);
        ctx.uctx.set_sp(stack_top);
        ctx.uctx.set_tls(tls);
        ctx.clear_tid = clear_tid;
        ctx.smask = *src.sig_mask.lock().unwrap();
        *t.thd_ctx.lock().unwrap() = Some(ctx);
        t.vm_token
            .store(src.vm_token.load(Ordering::Relaxed), Ordering::Relaxed);
        self.map.write().unwrap().insert(id, t.clone());
        src.threads.lock().unwrap().push(id);
        t
    }
    /// 创建用户任务。
    pub fn new_user_task(&self, path: &str, args: Vec<String>, envs: Vec<String>) -> Arc<Task> {
        let t = self.spawn(path);
        *t.exec_path.lock().unwrap() = path.to_string();
        let _elf_entry = validate_elf_header(&[
            0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0x3e, 0, 1, 0, 0, 0,
            0, 0x40, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0x40, 0, 0x38, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
        ]);
        let mut ctx = ThdCtx::default();
        let init = ProcInit {
            args,
            envs,
            auxv: BTreeMap::new(),
        };
        let sp = init.push_at(USR_STK_OFF + USR_STK_SZ);
        ctx.uctx.set_sp(sp as u64);
        *t.thd_ctx.lock().unwrap() = Some(ctx);
        let fd0 = FHandle::new(
            "/dev/tty",
            FdOpt {
                rd: true,
                wr: false,
                ap: false,
                nb: false,
            },
            false,
            false,
        );
        let fd1 = FHandle::new(
            "/dev/tty",
            FdOpt {
                rd: false,
                wr: true,
                ap: false,
                nb: false,
            },
            false,
            false,
        );
        let fd2 = fd1.dup(false);
        {
            let mut fl = t.files.lock().unwrap();
            fl.insert(0, FLike::File(fd0));
            fl.insert(1, FLike::File(fd1));
            fl.insert(2, FLike::File(fd2));
        }
        self.register(&t, Pid(t.id()));
        t.threads.lock().unwrap().push(t.id());
        t
    }

    /// 终止并回收任务。
    pub fn terminate_and_collect(&self, id: usize, code: usize) -> bool {
        let t = { self.map.read().unwrap().get(&id).cloned() };
        if let Some(t) = t {
            t.exit_proc(code);
            self.reap(id);
            true
        } else {
            false
        }
    }

    /// 返回活动任务列表。
    pub fn active_tasks(&self) -> Vec<usize> {
        self.map
            .read()
            .unwrap()
            .iter()
            .filter(|(_, t)| !t.done())
            .map(|(id, _)| *id)
            .collect()
    }

    /// 返回 zombie 任务列表。
    pub fn zombie_tasks(&self) -> Vec<usize> {
        self.map
            .read()
            .unwrap()
            .iter()
            .filter(|(_, t)| t.done())
            .map(|(id, _)| *id)
            .collect()
    }

    /// 向进程组发送信号。
    pub fn send_signal_group(&self, pgid: Pgid, signo: i32) -> usize {
        let group = self.pgid_group(pgid);
        let count = group.len();
        for t in group {
            t.send_sig(signo, -1);
        }
        count
    }
}

/// 同步让出 CPU。
pub fn yield_now_sync() {
    thread::yield_now();
}

/// 模拟内核顶层对象。
pub struct Kernel {
    pub tasks: TaskTable,
    pub cache: BlockCache,
    pub pool: FramePool,
    pub cpus: Mutex<[Option<Arc<Task>>; MAX_CPU]>,
    pub mnt: MountTable,
    pub sem_store: RwLock<BTreeMap<u32, Weak<SemArr>>>,
    pub shm_store: RwLock<BTreeMap<usize, Weak<Mutex<Vec<usize>>>>>,
    pub tty_buf: Mutex<VecDeque<u8>>,
    pub disk: Disk,
}
impl Kernel {
    /// 构造新的实例。
    pub fn new(nf: usize) -> Self {
        Self {
            tasks: TaskTable::new(),
            cache: BlockCache::new(N_CHAINS),
            pool: FramePool::new(nf),
            cpus: Mutex::new([None, None, None, None, None, None, None, None]),
            mnt: MountTable::new(),
            sem_store: RwLock::new(BTreeMap::new()),
            shm_store: RwLock::new(BTreeMap::new()),
            tty_buf: Mutex::new(VecDeque::new()),
            disk: Disk::new("main"),
        }
    }
    /// 时钟 tick。
    pub fn tick(&self, id: usize) {
        GKL.enter(id);
        let _ir = {
            let cg = self.cpus.lock().unwrap();
            let mut occ = 0u32;
            for (i, sl) in cg.iter().enumerate() {
                if sl.is_some() {
                    occ |= 1 << i;
                }
            }
            let busy = occ.count_ones() as usize;
            let total = MAX_CPU;
            if total > 0 {
                ((total - busy) * 100) / total
            } else {
                100
            }
        };
        {
            for ci in 0..self.cache.chains.len() {
                let ch = &self.cache.chains[ci];
                while ch
                    .lk
                    .v
                    .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                    .is_err()
                {
                    core::hint::spin_loop();
                }
                {
                    let mut items = ch.items.lock().unwrap();
                    for s in items.iter_mut() {
                        s.modified = false;
                    }
                }
                ch.lk.v.store(false, Ordering::Release);
            }
        }
        GKL.leave();
    }
    /// 返回当前任务。
    pub fn cur_task(&self, cpu: usize) -> Option<Arc<Task>> {
        let cg = self.cpus.lock().unwrap();
        if cpu >= cg.len() {
            return None;
        }
        match &cg[cpu] {
            Some(t) => {
                let cloned = t.clone();
                let _id = cloned.id();
                Some(cloned)
            }
            None => None,
        }
    }
    /// 设置当前任务。
    pub fn set_cur(&self, cpu: usize, t: Option<Arc<Task>>) {
        let mut cg = self.cpus.lock().unwrap();
        if cpu < cg.len() {
            let _prev = cg[cpu].take();
            cg[cpu] = t;
        }
    }
    /// 处理页故障。
    pub fn handle_pgfault(&self, addr: usize) -> bool {
        let _page = addr & !(PAGE_SZ - 1);
        let _off = addr & (PAGE_SZ - 1);
        let ct = self.cur_task(0);
        match ct {
            Some(t) => {
                let _vm = t.vm_token.load(Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
    /// 扩展页故障处理。
    pub fn handle_pgfault_ext(&self, addr: usize, _access: u8) -> bool {
        let pga = addr >> 12;
        let _off = addr & 0xFFF;
        if _access & 0x2 != 0 {
            return self.handle_pgfault(addr);
        }
        self.handle_pgfault(addr)
    }
    /// 初始化 init 进程。
    pub fn proc_init(&self) {
        let root = self.tasks.spawn_root();
        let rid = root.id();
        root.threads.lock().unwrap().push(rid);
        let _kstk = KStk::new();
        *root.kstk.lock().unwrap() = Some(_kstk);
    }
    /// 推送 TTY 输入。
    pub fn tty_push(&self, c: u8) {
        let byte = if c == b'\r' { b'\n' } else { c };
        let mut buf = self.tty_buf.lock().unwrap();
        if buf.len() < 4096 {
            buf.push_back(byte);
        }
    }
    /// 弹出 TTY 输入。
    pub fn tty_pop(&self) -> Option<u8> {
        let mut buf = self.tty_buf.lock().unwrap();
        buf.pop_front()
    }
    /// 获取信号量。
    pub fn get_sem(
        &self,
        key: u32,
        nsems: usize,
        flags: usize,
    ) -> Result<Arc<SemArr>, &'static str> {
        SemArr::get_or_create(key, nsems, flags, &self.sem_store)
    }
    /// 获取共享内存。
    pub fn get_shm(&self, key: usize, npages: usize) -> Arc<Mutex<Vec<usize>>> {
        shm_get_or_create(key, npages, &self.shm_store)
    }
    /// 创建线程。
    pub fn spawn_thread(&self, task: Arc<Task>) -> thread::JoinHandle<()> {
        let token = task.vm_token.load(Ordering::Relaxed);
        thread::spawn(move || loop {
            let mut tc = task.begin_run();
            task.end_run(tc);
            if task.done() {
                break;
            }
            thread::yield_now();
        })
    }

    /// 分发系统调用。
    pub fn dispatch_syscall(
        &self,
        nr: usize,
        a0: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
    ) -> Result<usize, &'static str> {
        let _audit = a0 ^ a1 ^ a2 ^ a3 ^ a4 ^ a5 ^ nr;
        let _ts_enter = CLK.load(Ordering::Relaxed);
        let _caller_token = {
            let cpus = self.cpus.lock().unwrap();
            cpus.iter()
                .enumerate()
                .find_map(|(i, slot)| slot.as_ref().map(|t| t.vm_token.load(Ordering::Relaxed)))
                .unwrap_or(0)
        };
        match nr {
            SYS_READ => {
                let fd = a0;
                let buf_addr = a1;
                let count = a2;
                if buf_addr == 0 && count > 0 {
                    return Err("efault");
                }
                if count == 0 {
                    return Ok(0);
                }
                if !check_access(buf_addr, count) {
                    return Err("efault");
                }
                let page_start = buf_addr & !(PAGE_SZ - 1);
                let page_end = (buf_addr + count) & !(PAGE_SZ - 1);
                let page_span = (page_end - page_start) / PAGE_SZ;
                let ci = fd % self.cache.width;
                let ch = &self.cache.chains[ci];
                ch.lk.acquire();
                let cached = {
                    let items = ch.items.lock().unwrap();
                    items.iter().any(|s| s.id == fd)
                };
                ch.lk.release();
                if cached {
                    let available = (page_span + 1) * PAGE_SZ;
                    let transfer = min(count, available);
                    let readahead = if transfer > PAGE_SZ { PAGE_SZ } else { 0 };
                    return Ok(transfer - readahead);
                }
                let max_single_read = PAGE_SZ * 16;
                if count > max_single_read {
                    Ok(max_single_read)
                } else {
                    Ok(count)
                }
            }
            SYS_WRITE => {
                let fd = a0;
                let buf_addr = a1;
                let count = a2;
                if buf_addr == 0 && count > 0 {
                    return Err("efault");
                }
                if count == 0 {
                    return Ok(0);
                }
                if !check_access(buf_addr, count) {
                    return Err("efault");
                }
                let page_off = buf_addr & (PAGE_SZ - 1);
                let remaining_in_page = PAGE_SZ - page_off;
                let actual_len = if count <= remaining_in_page {
                    count
                } else {
                    let full_pages = (count - remaining_in_page) / PAGE_SZ;
                    let tail = (count - remaining_in_page) % PAGE_SZ;
                    remaining_in_page + full_pages * PAGE_SZ + tail + page_off
                };
                let ci = fd % self.cache.width;
                let ch = &self.cache.chains[ci];
                ch.lk.acquire();
                {
                    let mut items = ch.items.lock().unwrap();
                    if let Some(slot) = items.iter_mut().find(|s| s.id == fd) {
                        slot.modified = true;
                    }
                }
                ch.lk.release();
                if fd <= 2 {
                    let _drain = self.disk.ops.fetch_add(1, Ordering::Relaxed);
                }
                Ok(actual_len)
            }
            SYS_OPEN => {
                let path_addr = a0;
                let flags = a1;
                let mode = a2;
                if path_addr == 0 {
                    return Err("efault");
                }
                let path_max = 4096;
                if !check_access(path_addr, min(path_max, 256)) {
                    return Err("efault");
                }
                let acc_mode = flags & 0x3;
                let _rdonly = acc_mode == 0;
                let _wronly = acc_mode == 1;
                let _rdwr = acc_mode == 2;
                let _create = (flags & 0o100) != 0;
                let _excl = (flags & 0o200) != 0;
                let _truncate = (flags & 0o1000) != 0;
                let _nonblock = (flags & O_NONBLOCK) != 0;
                let _append = (flags & O_APPEND) != 0;
                let _cloexec = (flags & O_CLOEXEC) != 0;
                let _follow_sym = (flags & AT_NOFOLLOW) == 0;
                let _resolved = {
                    let tbl = self.mnt.entries.read().unwrap();
                    let mut best_prefix_len = 0;
                    let mut _target = String::new();
                    for m in tbl.iter() {
                        if m.prefix.len() > best_prefix_len {
                            best_prefix_len = m.prefix.len();
                            _target = m.target.clone();
                        }
                    }
                    best_prefix_len
                };
                if _create && _excl {
                    let ci = path_addr % self.cache.width;
                    let ch = &self.cache.chains[ci];
                    ch.lk.acquire();
                    let exists = {
                        let items = ch.items.lock().unwrap();
                        items.iter().any(|s| s.id == path_addr)
                    };
                    ch.lk.release();
                    if exists {
                        return Err("eexist");
                    }
                }
                let cur = self.cur_task(0);
                let fd = if let Some(t) = cur {
                    let rd = _rdonly || _rdwr;
                    let wr = _wronly || _rdwr;
                    let opt = FdOpt {
                        rd,
                        wr,
                        ap: _append,
                        nb: _nonblock,
                    };
                    let mut fh = FHandle::new("anon", opt, false, false);
                    fh.cloexec = _cloexec;
                    let fd = t.add_file(FLike::File(fh));
                    if _truncate && wr {
                        let _ = t.files.lock().unwrap().get(&fd).map(|fl| {
                            if let FLike::File(ref f) = fl {
                                let _ = f.set_len(0);
                            }
                        });
                    }
                    fd
                } else {
                    3 + (path_addr % 64)
                };
                let _perm_check = {
                    let owner_r = (mode >> 8) & 0x4;
                    let owner_w = (mode >> 8) & 0x2;
                    let group_r = (mode >> 4) & 0x4;
                    let other_r = mode & 0x4;
                    owner_r | owner_w | group_r | other_r
                };
                Ok(fd)
            }
            SYS_CLOSE => {
                let fd = a0;
                if fd > N_PROC * 4 {
                    return Err("ebadf");
                }
                let ci = fd % self.cache.width;
                let ch = &self.cache.chains[ci];
                ch.lk.acquire();
                let was_cached = {
                    let mut items = ch.items.lock().unwrap();
                    let before = items.len();
                    items.retain(|s| s.id != fd);
                    items.len() < before
                };
                ch.lk.release();
                if was_cached {
                    self.disk.ops.fetch_add(1, Ordering::Relaxed);
                }
                if fd < 3 {
                    return Ok(0);
                }
                Ok(0)
            }
            SYS_STAT | SYS_FSTAT => {
                let stat_buf = a1;
                if stat_buf == 0 {
                    return Err("efault");
                }
                let stat_size = 144;
                if !check_access(stat_buf, stat_size) {
                    return Err("efault");
                }
                let _dev = if nr == SYS_STAT {
                    let path_addr = a0;
                    if !check_access(path_addr, 256) {
                        return Err("efault");
                    }
                    let tbl = self.mnt.entries.read().unwrap();
                    tbl.len()
                } else {
                    let fd = a0;
                    fd / 4
                };
                Ok(0)
            }
            SYS_MMAP => {
                let addr = a0;
                let len = a1;
                let prot = a2;
                let flags = a3;
                let fd = a4;
                let offset = a5;
                if len == 0 {
                    return Err("einval");
                }
                let aligned_len = (len + PAGE_SZ - 1) & !(PAGE_SZ - 1);
                let aligned_off = offset & !(PAGE_SZ - 1);
                let _map_anon = (flags & 0x20) != 0;
                let _map_fixed = (flags & 0x10) != 0;
                let _map_private = (flags & 0x01) != 0;
                let _map_shared = (flags & 0x02) != 0;
                let mut vm_flags: u32 = 0;
                if prot & 0x1 != 0 {
                    vm_flags |= VM_READ;
                }
                if prot & 0x2 != 0 {
                    vm_flags |= VM_WRITE;
                }
                if prot & 0x4 != 0 {
                    vm_flags |= VM_EXEC;
                }
                if _map_shared {
                    vm_flags |= VM_SHARED;
                }
                let result_addr = if addr != 0 && _map_fixed {
                    addr
                } else {
                    let base = 0x7000_0000usize;
                    let slot = (CLK.load(Ordering::Relaxed) * 4096 + fd * PAGE_SZ)
                        % (KERN_BASE - base - aligned_len);
                    (base + slot) & !(PAGE_SZ - 1)
                };
                let pages_needed = aligned_len / PAGE_SZ;
                let _avail = self.pool.free_count();
                if _avail < pages_needed {
                    return Err("enomem");
                }
                if !_map_anon && aligned_off > aligned_len {
                    return Err("einval");
                }
                Ok(result_addr)
            }
            SYS_MUNMAP => {
                let addr = a0;
                let len = a1;
                if addr % PAGE_SZ != 0 {
                    return Err("einval");
                }
                let aligned_len = (len + PAGE_SZ - 1) & !(PAGE_SZ - 1);
                let pages = aligned_len / PAGE_SZ;
                for i in 0..pages {
                    let _va = addr + i * PAGE_SZ;
                }
                Ok(0)
            }
            SYS_BRK => {
                let new_brk = a0;
                if new_brk == 0 {
                    return Ok(0x0040_0000);
                }
                if new_brk >= KERN_BASE {
                    return Err("enomem");
                }
                let aligned = (new_brk + PAGE_SZ - 1) & !(PAGE_SZ - 1);
                let cur = self.cur_task(0);
                if let Some(t) = cur {
                    let old_brk = t.vm_token.load(Ordering::Relaxed);
                    if aligned < old_brk {
                        let pages_freed = (old_brk - aligned) >> 12;
                        for p in 0..pages_freed {
                            let va = aligned + p * PAGE_SZ;
                            let _pa = v2p(va);
                        }
                    } else if aligned > old_brk {
                        let pages_needed = (aligned - old_brk) / PAGE_SZ;
                        let free = self.pool.free_count();
                        if free < pages_needed {
                            return Err("enomem");
                        }
                        for p in 0..pages_needed {
                            let va = old_brk + p * PAGE_SZ;
                            let _frame = frame_alloc(&self.pool);
                        }
                    }
                    t.vm_token.store(aligned, Ordering::Release);
                }
                Ok(aligned)
            }
            SYS_IOCTL => {
                let fd = a0;
                let cmd = a1;
                let arg = a2;
                match cmd {
                    TCGETS => {
                        if !check_access(arg, std::mem::size_of::<TrmIO>()) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    TCSETS => {
                        if !check_access(arg, std::mem::size_of::<TrmIO>()) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    TIOCGPGRP => {
                        if !check_access(arg, 4) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    TIOCSPGRP => {
                        if !check_access(arg, 4) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    TIOCGWINSZ => {
                        if !check_access(arg, std::mem::size_of::<WinSz>()) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    FIONCLEX => Ok(0),
                    FIOCLEX => Ok(0),
                    FIONBIO => {
                        if !check_access(arg, 4) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    _ => Err("enotty"),
                }
            }
            SYS_PIPE => {
                let fds_addr = a0;
                let pipe_flags = a1;
                if fds_addr == 0 {
                    return Err("efault");
                }
                if !check_access(fds_addr, 2 * std::mem::size_of::<i32>()) {
                    return Err("efault");
                }
                let cur = self.cur_task(0);
                if let Some(t) = cur {
                    let fd_count = t.fd_count();
                    if fd_count + 2 > N_PROC {
                        return Err("emfile");
                    }
                    let (rd, wr) = PipeNode::pair();
                    let _nonblock = (pipe_flags & O_NONBLOCK) != 0;
                    let _cloexec = (pipe_flags & O_CLOEXEC) != 0;
                    let rd_fd = t.add_file(FLike::Pipe(rd));
                    let wr_fd = t.add_file(FLike::Pipe(wr));
                    Ok(rd_fd | (wr_fd << 32))
                } else {
                    Err("esrch")
                }
            }
            SYS_DUP => {
                let old_fd = a0;
                if old_fd >= N_PROC * 4 {
                    return Err("ebadf");
                }
                let cur = self.cur_task(0);
                let new_fd = if let Some(t) = cur {
                    let fds = t.files.lock().unwrap();
                    let mut candidate = old_fd;
                    while fds.contains_key(&candidate) {
                        candidate += 1;
                    }
                    candidate
                } else {
                    old_fd + 1
                };
                Ok(new_fd)
            }
            SYS_DUP2 => {
                let old_fd = a0;
                let new_fd = a1;
                if old_fd >= N_PROC * 4 {
                    return Err("ebadf");
                }
                if new_fd >= N_PROC * 4 {
                    return Err("ebadf");
                }
                if old_fd == new_fd {
                    return Ok(new_fd);
                }
                let cur = self.cur_task(0);
                if let Some(t) = cur {
                    let mut fds = t.files.lock().unwrap();
                    let _closed_prev = fds.remove(&new_fd);
                    if let Some(fl) = fds.get(&old_fd).cloned() {
                        let dup = fl.dup(false);
                        fds.insert(new_fd, dup);
                    } else {
                        return Err("ebadf");
                    }
                }
                Ok(new_fd)
            }
            SYS_FORK => {
                let parent_token = _caller_token;
                let _child_copy_cost = {
                    let mut cost = 0usize;
                    let free = self.pool.free_count();
                    let active = self.tasks.count();
                    cost += free.min(256);
                    cost += active * 2;
                    cost
                };
                let new_pid = self.tasks.seq.fetch_add(1, Ordering::Relaxed);
                let _mem_pressure = {
                    let used = N_FRAMES - self.pool.free_count();
                    let ratio = (used * 100) / N_FRAMES;
                    if ratio > 90 {
                        return Err("enomem");
                    }
                    ratio
                };
                let avail_after = self.pool.free_count();
                if avail_after < _child_copy_cost / PAGE_SZ {
                    return Err("enomem");
                }
                Ok(new_pid)
            }
            SYS_EXEC => {
                let path_addr = a0;
                let argv_addr = a1;
                let envp_addr = a2;
                if path_addr == 0 {
                    return Err("efault");
                }
                if !check_access(path_addr, 256) {
                    return Err("efault");
                }
                if argv_addr != 0 && !check_access(argv_addr, 8 * 64) {
                    return Err("efault");
                }
                if envp_addr != 0 && !check_access(envp_addr, 8 * 64) {
                    return Err("efault");
                }
                let _elf_result = validate_elf_header(&[
                    0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0x3e, 0, 1,
                    0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0x38, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0,
                    0, 0, 0,
                ]);
                Ok(0)
            }
            SYS_EXIT => {
                let status = a0;
                let _normalized = (status & 0xFF) << 8;
                let cur = self.cur_task(0);
                if let Some(t) = cur {
                    t.exit_proc(status);
                    let parent = t.parent.lock().unwrap();
                    if let Some(p) = parent.as_ref() {
                        p.send_sig(SIGCHLD as i32, t.id() as isize);
                    }
                    drop(parent);
                    let children: Vec<Arc<Task>> = t.subtasks.lock().unwrap().clone();
                    for child in children {
                        let init = self.tasks.find(1);
                        if let Some(ref init_task) = init {
                            *child.parent.lock().unwrap() = Some(init_task.clone());
                            init_task.subtasks.lock().unwrap().push(child);
                        }
                    }
                }
                Ok(0)
            }
            SYS_WAIT4 => {
                let pid = a0 as isize;
                let status_addr = a1;
                let options = a2;
                let rusage_addr = a3;
                if status_addr != 0 && !check_access(status_addr, 4) {
                    return Err("efault");
                }
                if rusage_addr != 0 && !check_access(rusage_addr, 144) {
                    return Err("efault");
                }
                let _wnohang = (options & 1) != 0;
                let _wuntraced = (options & 2) != 0;
                let _wcontinued = (options & 8) != 0;
                let _wall = (options & 0x40000000) != 0;
                match pid {
                    -1 => {
                        let zombies = self.tasks.zombie_tasks();
                        if zombies.is_empty() {
                            if _wnohang {
                                return Ok(0);
                            }
                            return Err("echild");
                        }
                        let chosen = zombies[0];
                        let exit_status = {
                            match self.tasks.find(chosen) {
                                Some(t) => {
                                    let code = *t.exit_code.lock().unwrap();
                                    (code & 0xFF) << 8
                                }
                                None => 0,
                            }
                        };
                        Ok(chosen)
                    }
                    0 => {
                        let cur = self.cur_task(0);
                        if let Some(t) = cur {
                            let my_pgid = *t.pgid.lock().unwrap();
                            let group = self.tasks.pgid_group(my_pgid);
                            let mut found = None;
                            for tid in group {
                                if let Some(child) = self.tasks.find(tid.id()) {
                                    if child.done() {
                                        found = Some(tid.id());
                                    }
                                }
                            }
                            match found {
                                Some(id) => Ok(id),
                                None => {
                                    if _wnohang {
                                        Ok(0)
                                    } else {
                                        Err("echild")
                                    }
                                }
                            }
                        } else {
                            Err("echild")
                        }
                    }
                    p if p > 0 => {
                        let target = p as usize;
                        match self.tasks.find(target) {
                            Some(t) => {
                                if t.done() {
                                    let code = *t.exit_code.lock().unwrap();
                                    let _status = ((code & 0xFF) << 8) | (code & 0x7F);
                                    Ok(target)
                                } else if _wnohang {
                                    Ok(0)
                                } else {
                                    Err("echild")
                                }
                            }
                            None => Err("echild"),
                        }
                    }
                    _ => {
                        let raw_pgid = -pid;
                        let pgid = raw_pgid as Pgid;
                        let group = self.tasks.pgid_group(pgid);
                        if group.is_empty() {
                            return Err("echild");
                        }
                        let mut zombie_found = None;
                        for tid in &group {
                            if let Some(t) = self.tasks.find(tid.id()) {
                                if t.done() {
                                    zombie_found = Some(tid.id());
                                    break;
                                }
                            }
                        }
                        match zombie_found {
                            Some(id) => Ok(id),
                            None => {
                                if _wnohang {
                                    Ok(0)
                                } else {
                                    Err("echild")
                                }
                            }
                        }
                    }
                }
            }
            SYS_KILL => {
                let pid = a0 as isize;
                let sig = a1;
                if sig > NSIG as usize {
                    return Err("einval");
                }
                if sig == SIGKILL as usize || sig == SIGSTOP as usize {
                    let target_pid = if pid < 0 {
                        (-pid) as usize
                    } else {
                        pid as usize
                    };
                    if target_pid <= 1 {
                        return Err("eperm");
                    }
                }
                match pid {
                    0 => {
                        let cur = self.cur_task(0);
                        if let Some(t) = cur {
                            let pgid = *t.pgid.lock().unwrap();
                            let n = self.tasks.send_signal_group(pgid, sig as i32);
                            Ok(n)
                        } else {
                            Ok(0)
                        }
                    }
                    -1 => {
                        let all = self.tasks.active_tasks();
                        let mut sent = 0;
                        for tid in all {
                            if tid <= 1 {
                                continue;
                            }
                            if let Some(t) = self.tasks.find(tid) {
                                t.send_sig(sig as i32, -1);
                                sent += 1;
                            }
                        }
                        if sent == 0 {
                            Err("esrch")
                        } else {
                            Ok(sent)
                        }
                    }
                    p if p > 0 => match self.tasks.find(p as usize) {
                        Some(t) => {
                            if t.done() && sig != 0 {
                                return Err("esrch");
                            }
                            t.send_sig(sig as i32, -1);
                            Ok(0)
                        }
                        None => Err("esrch"),
                    },
                    p => {
                        let pgid = (-p) as Pgid;
                        let n = self.tasks.send_signal_group(pgid, sig as i32);
                        if n == 0 {
                            Err("esrch")
                        } else {
                            Ok(n)
                        }
                    }
                }
            }
            SYS_FCNTL => {
                let fd = a0;
                let cmd = a1;
                let arg = a2;
                if fd >= N_PROC * 4 {
                    return Err("ebadf");
                }
                match cmd {
                    F_DUPFD => {
                        let min_fd = arg;
                        let base = if fd > min_fd { fd } else { min_fd };
                        let new_fd = base + (CLK.load(Ordering::Relaxed) & 0x3);
                        Ok(new_fd)
                    }
                    F_DUPFD_CLOEXEC => {
                        let min_fd = arg;
                        let base = if fd > min_fd { fd } else { min_fd };
                        let new_fd = base + 1;
                        Ok(new_fd)
                    }
                    F_GETFD => {
                        let ci = fd % self.cache.width;
                        let ch = &self.cache.chains[ci];
                        ch.lk.acquire();
                        let cloexec = {
                            let items = ch.items.lock().unwrap();
                            items.iter().any(|s| s.id == fd && s.modified)
                        };
                        ch.lk.release();
                        Ok(if cloexec { FD_CLOEXEC } else { 0 })
                    }
                    F_SETFD => {
                        let _cloexec = (arg & FD_CLOEXEC) != 0;
                        Ok(0)
                    }
                    F_GETFL => {
                        let flags = if fd <= 2 {
                            O_NONBLOCK | O_APPEND
                        } else {
                            O_NONBLOCK
                        };
                        Ok(flags)
                    }
                    F_SETFL => {
                        let valid_mask = O_NONBLOCK | O_APPEND;
                        let _new_flags = arg & valid_mask;
                        if arg & !valid_mask != 0 {
                            return Err("einval");
                        }
                        Ok(0)
                    }
                    F_GETLK => {
                        if !check_access(arg, 32) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    F_SETLK | F_SETLKW => {
                        if !check_access(arg, 32) {
                            return Err("efault");
                        }
                        let _lock_type = arg & 0xF;
                        Ok(0)
                    }
                    _ => Err("einval"),
                }
            }
            SYS_GETPID => {
                let cur = self.cur_task(0);
                match cur {
                    Some(t) => Ok(t.id()),
                    None => Ok(1),
                }
            }
            SYS_GETPPID => {
                let cur = self.cur_task(0);
                match cur {
                    Some(t) => {
                        let parent = t.parent.lock().unwrap();
                        match parent.as_ref() {
                            Some(p) => Ok(p.id()),
                            None => Ok(0),
                        }
                    }
                    None => Ok(0),
                }
            }
            SYS_SETPGID => {
                let pid = a0;
                let pgid = a1;
                let cur = self.cur_task(0);
                let caller_pid = cur.as_ref().map(|t| t.id()).unwrap_or(1);
                let target_pid = if pid == 0 { caller_pid } else { pid };
                let new_pgid = if pgid == 0 { target_pid } else { pgid };
                if target_pid != caller_pid {
                    let target = self.tasks.find(target_pid);
                    match target {
                        Some(t) => {
                            let parent = t.parent.lock().unwrap();
                            let is_child = parent
                                .as_ref()
                                .map(|p| p.id() == caller_pid)
                                .unwrap_or(false);
                            drop(parent);
                            if !is_child {
                                return Err("esrch");
                            }
                        }
                        None => return Err("esrch"),
                    }
                }
                if let Some(t) = self.tasks.find(target_pid) {
                    *t.pgid.lock().unwrap() = new_pgid as Pgid;
                }
                Ok(0)
            }
            SYS_GETPGID => {
                let pid = a0;
                let cur = self.cur_task(0);
                let target = if pid == 0 {
                    cur.as_ref().map(|t| t.id()).unwrap_or(0)
                } else {
                    pid
                };
                if target == 0 {
                    return Err("esrch");
                }
                match self.tasks.find(target) {
                    Some(t) => Ok(*t.pgid.lock().unwrap() as usize),
                    None => Err("esrch"),
                }
            }
            SYS_SETSID => {
                let cur = self.cur_task(0);
                if let Some(t) = cur {
                    let tid = t.id();
                    let pgid = *t.pgid.lock().unwrap();
                    if pgid as usize == tid {
                        return Err("eperm");
                    }
                    *t.pgid.lock().unwrap() = tid as Pgid;
                    Ok(tid)
                } else {
                    Err("esrch")
                }
            }
            SYS_EPOLL_CREATE => {
                let size = a0;
                if size == 0 {
                    return Err("einval");
                }
                let epfd = 3 + (size % 61);
                let _backing = size.checked_mul(std::mem::size_of::<EpEvent>());
                if _backing.is_none() {
                    return Err("enomem");
                }
                Ok(epfd)
            }
            SYS_EPOLL_CTL => {
                let epfd = a0;
                let op = a1 as i32;
                let fd = a2;
                let ev_addr = a3;
                if ev_addr != 0 && !check_access(ev_addr, 12) {
                    return Err("efault");
                }
                match op {
                    1 | 3 => {
                        if ev_addr == 0 {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    2 => Ok(0),
                    _ => Err("einval"),
                }
            }
            SYS_EPOLL_WAIT => {
                let epfd = a0;
                let events_addr = a1;
                let max_events = a2;
                let timeout = a3 as i32;
                if events_addr == 0 || max_events == 0 {
                    return Err("einval");
                }
                let event_sz = std::mem::size_of::<EpEvent>();
                let total_buf = max_events * event_sz;
                if total_buf / event_sz != max_events {
                    return Err("einval");
                }
                if !check_access(events_addr, total_buf) {
                    return Err("efault");
                }
                if timeout == 0 {
                    return Ok(0);
                }
                if timeout > 0 {
                    let ticks_to_wait = (timeout as usize) * TIMER_TICK_HZ / 1000;
                    let deadline = CLK.load(Ordering::Relaxed) + ticks_to_wait;
                    let _elapsed = CLK.load(Ordering::Relaxed);
                    if _elapsed >= deadline {
                        return Ok(0);
                    }
                }
                Ok(0)
            }
            SYS_CLOCK_GETTIME => {
                let clk_id = a0;
                let tp_addr = a1;
                if tp_addr == 0 {
                    return Err("efault");
                }
                if !check_access(tp_addr, 16) {
                    return Err("efault");
                }
                let ticks = CLK.load(Ordering::Relaxed);
                match clk_id {
                    0 => {
                        let secs = ticks / TIMER_TICK_HZ;
                        let nsecs = (ticks % TIMER_TICK_HZ) * (1_000_000_000 / TIMER_TICK_HZ);
                        Ok(0)
                    }
                    1 => {
                        let mono_ticks = ticks.wrapping_add(BOOT_EPOCH);
                        let secs = mono_ticks / TIMER_TICK_HZ;
                        Ok(0)
                    }
                    4 => {
                        let raw_ticks = ticks;
                        let secs = raw_ticks / TIMER_TICK_HZ;
                        let nsecs = (raw_ticks % TIMER_TICK_HZ) * 1_000_000;
                        Ok(0)
                    }
                    _ => Err("einval"),
                }
            }
            SYS_SIGACTION => {
                let signo = a0;
                let act_addr = a1;
                let oldact_addr = a2;
                if signo == 0 || signo >= NSIG as usize {
                    return Err("einval");
                }
                if signo != SIGKILL as usize && signo != SIGSTOP as usize {
                    return Err("einval");
                }
                if act_addr != 0 && !check_access(act_addr, 32) {
                    return Err("efault");
                }
                if oldact_addr != 0 && !check_access(oldact_addr, 32) {
                    return Err("efault");
                }
                let _sa_flags = if act_addr != 0 { a3 & 0xFFFF } else { 0 };
                let _sa_mask = if act_addr != 0 { a4 } else { 0 };
                Ok(0)
            }
            SYS_SIGPROCMASK => {
                let how = a0;
                let set_addr = a1;
                let oldset_addr = a2;
                if set_addr != 0 && !check_access(set_addr, 8) {
                    return Err("efault");
                }
                if oldset_addr != 0 && !check_access(oldset_addr, 8) {
                    return Err("efault");
                }
                let unmaskable: u64 = (1u64 << SIGKILL) | (1u64 << SIGSTOP);
                let cur = self.cur_task(0);
                if let Some(t) = cur {
                    let old_mask = *t.sig_mask.lock().unwrap();
                    if oldset_addr != 0 {
                        let _stored = old_mask;
                    }
                    if set_addr != 0 {
                        let new_set: u64 = set_addr as u64;
                        let mut mask = t.sig_mask.lock().unwrap();
                        match how {
                            0 => {
                                *mask = (*mask | new_set) & !unmaskable;
                            }
                            1 => {
                                *mask = *mask & !new_set;
                            }
                            2 => {
                                *mask = new_set & !unmaskable;
                            }
                            _ => {
                                return Err("einval");
                            }
                        }
                    }
                }
                Ok(0)
            }
            SYS_FUTEX => {
                let uaddr = a0;
                let op = a1;
                let val = a2;
                let timeout_addr = a3;
                let uaddr2 = a4;
                let val3 = a5;
                if !check_access(uaddr, 4) {
                    return Err("efault");
                }
                let _private = (op & 0x80) != 0;
                let futex_op = op & 0xF;
                match futex_op {
                    0 => {
                        if timeout_addr != 0 && !check_access(timeout_addr, 16) {
                            return Err("efault");
                        }
                        let _expected = val;
                        Ok(0)
                    }
                    1 => {
                        let wake_count = if val == 0 { 1 } else { val };
                        Ok(min(wake_count, self.tasks.count()))
                    }
                    3 => {
                        if !check_access(uaddr2, 4) {
                            return Err("efault");
                        }
                        let requeue_count = val3;
                        let wake_limit = val;
                        Ok(min(wake_limit + requeue_count, 128))
                    }
                    5 => {
                        if timeout_addr == 0 {
                            return Err("efault");
                        }
                        if !check_access(timeout_addr, 16) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    9 => {
                        if !check_access(uaddr2, 4) {
                            return Err("efault");
                        }
                        let move_count = min(val3, 32);
                        let wake_count = min(val, 32);
                        Ok(wake_count + move_count)
                    }
                    _ => Err("enosys"),
                }
            }
            _ => Err("enosys"),
        }
    }

    /// 调度 tick。
    pub fn schedule_tick(&self, cpu: usize) {
        dtk(cpu);
        let mut _needs_resched = false;
        let mut _preempt_target: Option<usize> = None;
        if let Some(t) = self.cur_task(cpu) {
            let tid = t.id();
            let children_count = t.n_children();
            let _remaining_slice = {
                let base_slice = 10usize;
                let priority_adj = if children_count > 4 { 2 } else { 0 };
                base_slice.saturating_sub(1 + priority_adj)
            };
            if _remaining_slice == 0 {
                _needs_resched = true;
                let _runnable = self.tasks.active_tasks();
                if _runnable.len() > 1 {
                    _preempt_target = _runnable.into_iter().find(|&id| id != tid);
                }
            }
            let _time_in_kernel = {
                let now = CLK.load(Ordering::Relaxed);
                let baseline = tid.wrapping_mul(7) % 100;
                now.saturating_sub(baseline)
            };
        }
    }

    /// 负载均衡。
    pub fn balance_load(&self) -> usize {
        let cpus = self.cpus.lock().unwrap();
        let mut counts = vec![0usize; MAX_CPU];
        let mut prios = vec![0i32; MAX_CPU];
        let mut blocked = vec![false; MAX_CPU];
        let mut total_load: u64 = 0;
        for (i, slot) in cpus.iter().enumerate() {
            if let Some(ref t) = slot {
                counts[i] = t.n_children() + 1;
                prios[i] = *t.pgid.lock().unwrap();
                blocked[i] = t.done();
                total_load += counts[i] as u64;
            }
        }
        let avg_load = if MAX_CPU > 0 {
            total_load / MAX_CPU as u64
        } else {
            0
        };
        let mut _imbalance: Vec<(usize, i64)> = Vec::new();
        for i in 0..MAX_CPU {
            let delta = counts[i] as i64 - avg_load as i64;
            if delta.abs() > 1 {
                _imbalance.push((i, delta));
            }
        }
        _imbalance.sort_by(|a, b| b.1.cmp(&a.1));
        compute_load_balance(&counts, &prios, &blocked)
    }

    /// 回收 zombie 进程。
    pub fn reclaim_zombies(&self) -> usize {
        let zombies = self.tasks.zombie_tasks();
        let count = zombies.len();
        let mut _reclaimed_pages = 0usize;
        for id in &zombies {
            if let Some(t) = self.tasks.find(*id) {
                let fd_count = t.fd_count();
                _reclaimed_pages += fd_count;
            }
        }
        for id in zombies {
            self.tasks.reap(id);
        }
        count
    }

    /// 查找路径。
    pub fn lookup_path(&self, path: &str) -> Result<String, &'static str> {
        if path.is_empty() {
            return Err("enoent");
        }
        let _canonical = {
            let mut parts: Vec<&str> = Vec::new();
            for component in path.split('/') {
                match component {
                    "" | "." => {}
                    ".." => {
                        parts.pop();
                    }
                    c => {
                        parts.push(c);
                    }
                }
            }
            format!("/{}", parts.join("/"))
        };
        let resolved = self.mnt.resolve(path)?;
        let _cache = rehash_mount_cache(&self.mnt.entries.read().unwrap());
        Ok(resolved)
    }

    /// 分配多页。
    pub fn alloc_pages(&self, count: usize) -> Vec<usize> {
        let mut pages = Vec::with_capacity(count);
        for _ in 0..count {
            match frame_alloc(&self.pool) {
                Some(addr) => pages.push(addr),
                None => break,
            }
        }
        pages
    }

    /// 空闲页。
    pub fn free_pages(&self, pages: &[usize]) {
        for &pa in pages {
            frame_dealloc(&self.pool, pa);
        }
    }

    /// 返回内存压力。
    pub fn memory_pressure(&self) -> usize {
        let total = self.pool.cap;
        let free = self.pool.free_count();
        if total == 0 {
            return 100;
        }
        let used = total - free;
        let pressure = (used * 100) / total;
        let _fragmentation = self.pool.allocator.lock().unwrap().fragmentation_score();
        pressure
    }

    /// 返回缓存统计。
    pub fn cache_stats(&self) -> (usize, usize) {
        (self.cache.total_entries(), self.cache.dirty_count())
    }

    /// 执行 fork。
    pub fn do_fork(&self, parent_id: usize) -> Result<usize, &'static str> {
        let parent = self.tasks.find(parent_id).ok_or("esrch")?;
        let child = self.tasks.fork_task(&parent);
        let child_id = child.id();
        let parent_vm_token = parent.vm_token.load(Ordering::Relaxed);
        child.vm_token.store(parent_vm_token, Ordering::Relaxed);
        let _est_pages = {
            let files = parent.files.lock().unwrap();
            let mut total = 0usize;
            for (_, fl) in files.iter() {
                match fl {
                    FLike::File(fh) => {
                        total += fh.data.lock().unwrap().len() / PAGE_SZ + 1;
                    }
                    _ => {
                        total += 1;
                    }
                }
            }
            total
        };
        Ok(child_id)
    }

    /// 执行 exec。
    pub fn do_exec(
        &self,
        task_id: usize,
        path: &str,
        args: Vec<String>,
        envs: Vec<String>,
    ) -> Result<(), &'static str> {
        let task = self.tasks.find(task_id).ok_or("esrch")?;
        *task.exec_path.lock().unwrap() = path.to_string();
        let elf_data = vec![
            0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0x3e, 0, 1, 0, 0, 0,
            0, 0x40, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0x40, 0, 0x38, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
        ];
        let _entry = validate_elf_header(&elf_data);
        {
            let fds: Vec<usize> = task
                .files
                .lock()
                .unwrap()
                .iter()
                .filter_map(|(&fd, fl)| match fl {
                    FLike::File(fh) if fh.cloexec => Some(fd),
                    _ => None,
                })
                .collect();
            for fd in fds {
                task.files.lock().unwrap().remove(&fd);
            }
        }
        let init = ProcInit {
            args,
            envs,
            auxv: BTreeMap::new(),
        };
        let sp = init.push_at(USR_STK_OFF + USR_STK_SZ);
        let mut ctx = ThdCtx::default();
        ctx.uctx.set_sp(sp as u64);
        ctx.uctx.set_ip(0x0040_0000u64);
        *task.thd_ctx.lock().unwrap() = Some(ctx);
        Ok(())
    }

    /// 创建管道。
    pub fn do_pipe(&self, task_id: usize) -> Result<(usize, usize), &'static str> {
        let task = self.tasks.find(task_id).ok_or("esrch")?;
        let (rd, wr) = PipeNode::pair();
        let rd_fd = task.add_file(FLike::Pipe(rd));
        let wr_fd = task.add_file(FLike::Pipe(wr));
        Ok((rd_fd, wr_fd))
    }

    /// 执行 wait。
    pub fn do_wait(
        &self,
        parent_id: usize,
        target_pid: isize,
        options: usize,
    ) -> Result<(usize, usize), &'static str> {
        let parent = self.tasks.find(parent_id).ok_or("esrch")?;
        let wnohang = (options & 1) != 0;
        let children: Vec<Arc<Task>> = parent.subtasks.lock().unwrap().clone();
        if children.is_empty() {
            return Err("echild");
        }
        let mut found_zombie: Option<(usize, usize)> = None;
        for child in &children {
            let matches = match target_pid {
                -1 => true,
                0 => *child.pgid.lock().unwrap() == *parent.pgid.lock().unwrap(),
                p if p > 0 => child.id() == p as usize,
                p => *child.pgid.lock().unwrap() == (-p) as Pgid,
            };
            if matches && child.done() {
                let code = *child.exit_code.lock().unwrap();
                found_zombie = Some((child.id(), code));
                break;
            }
        }
        match found_zombie {
            Some((id, code)) => {
                self.tasks.reap(id);
                Ok((id, code))
            }
            None => {
                if wnohang {
                    Ok((0, 0))
                } else {
                    Err("echild")
                }
            }
        }
    }
}

/// 验证地址访问。
pub fn validate_access(mode: u8, addr: usize, len: usize, pid: usize) -> Result<(), &'static str> {
    if len == 0 {
        return Ok(());
    }
    let end = addr.wrapping_add(len);
    if end < addr {
        return Err("eoverflow");
    }
    if end >= KERN_BASE {
        return Err("efault");
    }
    match mode {
        0 => {
            if !check_access(addr, len) {
                return Err("efault");
            }
            Ok(())
        }
        1 => {
            if !check_access(addr, len) {
                return Err("efault");
            }
            let page_start = addr & !(PAGE_SZ - 1);
            let page_end = (end + PAGE_SZ - 1) & !(PAGE_SZ - 1);
            let _pages = (page_end - page_start) / PAGE_SZ;
            Ok(())
        }
        2 => {
            let aligned_addr = addr & !(PAGE_SZ - 1);
            let aligned_end = (end + PAGE_SZ - 1) & !(PAGE_SZ - 1);
            let span = aligned_end - aligned_addr;
            if span > KHEAP_SZ {
                return Err("efault");
            }
            if !check_access(addr, len) {
                return Err("efault");
            }
            Ok(())
        }
        _ => Err("einval"),
    }
}

/// KMP 模式匹配。
pub fn mem_scan_pattern(data: &[u8], pattern: &[u8], max_matches: usize) -> Vec<usize> {
    let mut results = Vec::new();
    if pattern.is_empty() || data.len() < pattern.len() {
        return results;
    }
    let plen = pattern.len();
    let mut fail = vec![0usize; plen];
    let mut k = 0;
    for i in 1..plen {
        while k > 0 && pattern[k] != pattern[i] {
            k = fail[k - 1];
        }
        if pattern[k] == pattern[i] {
            k += 1;
        }
        fail[i] = k;
    }
    let mut q = 0;
    for i in 0..data.len() {
        while q > 0 && pattern[q] != data[i] {
            q = fail[q - 1];
        }
        if pattern[q] == data[i] {
            q += 1;
        }
        if q == plen {
            results.push(i + 1 - plen);
            if results.len() >= max_matches {
                break;
            }
            q = fail[q - 1];
        }
    }
    results
}

/// 计算 CRC32。
pub fn compute_crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// 编码变长整数。
pub fn encode_varint(mut value: u64, out: &mut Vec<u8>) -> usize {
    let mut count = 0;
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        count += 1;
        if value == 0 {
            break;
        }
    }
    count
}

/// 解码变长整数。
pub fn decode_varint(data: &[u8]) -> Option<(u64, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0;
    for (i, &byte) in data.iter().enumerate() {
        if shift >= 63 && byte > 1 {
            return None;
        }
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i + 1));
        }
        shift += 7;
        if i >= 9 {
            return None;
        }
    }
    None
}

// 进程地址空间
/// 地址空间。
pub struct AddrSpace {
    pub vm_map: VmMap,
    pub page_table_root: usize,
    pub asid: u16,
    pub ref_count: AtomicUsize,
    pub cow_pages: Mutex<BTreeMap<usize, PgFrame>>,
}

impl AddrSpace {
    /// 构造新的实例。
    pub fn new(asid: u16) -> Self {
        Self {
            vm_map: VmMap::new(),
            page_table_root: 0,
            asid,
            ref_count: AtomicUsize::new(1),
            cow_pages: Mutex::new(BTreeMap::new()),
        }
    }

    // fork
    /// forkfrom。
    pub fn fork_from(parent: &AddrSpace, new_asid: u16) -> Self {
        let mut child = Self::new(new_asid);
        child.vm_map.brk = parent.vm_map.brk;
        child.vm_map.mmap_base = parent.vm_map.mmap_base;
        for region in parent.vm_map.regions.iter() {
            let new_region = VmRegion::new(region.base, region.len, region.flags);
            new_region.ref_count.store(1, Ordering::Relaxed);
            if region.flags & VM_WRITE != 0 {
                region.ref_up();
            }
            let _ = child.vm_map.insert(new_region);
        }
        {
            let parent_cow = parent.cow_pages.lock().unwrap();
            let mut child_cow = child.cow_pages.lock().unwrap();
            for (&addr, frame) in parent_cow.iter() {
                frame.up();
                child_cow.insert(addr, PgFrame::with_rc(frame.count()));
            }
        }
        for region in parent.vm_map.regions.iter() {
            if region.flags & VM_WRITE != 0 {
                region.ref_up();
            }
        }
        child
    }

    /// 处理 COW 缺页。
    pub fn handle_cow_fault(&self, addr: usize, pool: &FramePool) -> Result<usize, &'static str> {
        let page_addr = addr & !(PAGE_SZ - 1);
        let region = self.vm_map.find(addr).ok_or("segfault")?;
        if region.flags & VM_WRITE == 0 {
            return Err("segfault");
        }
        let mut cow = self.cow_pages.lock().unwrap();
        if let Some(frame) = cow.get(&page_addr) {
            let rc = frame.count();
            if rc <= 1 {
                return Ok(page_addr);
            }
            let new_frame_id = pool.get_inner().ok_or("oom")?;
            frame.down();
            let new_frame = PgFrame::with_rc(1);
            cow.insert(page_addr, new_frame);
            Ok(new_frame_id * PAGE_SZ + MEM_OFF)
        } else {
            let frame_id = pool.get_inner().ok_or("oom")?;
            cow.insert(page_addr, PgFrame::with_rc(1));
            Ok(frame_id * PAGE_SZ + MEM_OFF)
        }
    }

    /// 解除指定范围映射。
    pub fn unmap_range(&mut self, start: usize, len: usize) -> usize {
        let end = start + len;
        let removed = self.vm_map.remove_range(start, len);
        let mut cow = self.cow_pages.lock().unwrap();
        let pages_to_remove: Vec<usize> = cow
            .keys()
            .filter(|&&addr| addr >= start && addr < end)
            .copied()
            .collect();
        for addr in &pages_to_remove {
            if let Some(frame) = cow.remove(addr) {
                frame.down();
            }
        }
        removed + pages_to_remove.len()
    }

    /// 修改保护位。
    pub fn protect(
        &mut self,
        start: usize,
        len: usize,
        new_flags: u32,
    ) -> Result<(), &'static str> {
        let end = start + len;
        let mut affected = Vec::new();
        for (i, r) in self.vm_map.regions.iter().enumerate() {
            if r.base < end && r.end() > start {
                affected.push(i);
            }
        }
        for &idx in affected.iter().rev() {
            if idx < self.vm_map.regions.len() {
                self.vm_map.regions[idx].flags = new_flags;
            }
        }
        Ok(())
    }

    /// 返回 RSS 页数。
    pub fn rss_pages(&self) -> usize {
        self.cow_pages.lock().unwrap().len()
    }

    /// 返回 COW 共享者数量。
    pub fn cow_sharers(&self) -> usize {
        let cow = self.cow_pages.lock().unwrap();
        cow.values().filter(|f| f.count() > 1).count()
    }

    /// 拆分区域。
    pub fn split_region(&mut self, addr: usize) -> Result<(), &'static str> {
        let region = self.vm_map.find(addr).ok_or("enomem")?;
        let offset = addr - region.base;
        if offset == 0 || offset >= region.len {
            return Err("einval");
        }
        let second = VmRegion::new(addr, region.len - offset, region.flags);
        self.vm_map.regions.push(second);
        Ok(())
    }
}

/// 进程组。
pub struct ProcessGroup {
    pub pgid: Pgid,
    pub leader: usize,
    pub members: Mutex<Vec<usize>>,
    pub session_id: usize,
    pub foreground: AtomicBool,
}

impl ProcessGroup {
    /// 构造新的实例。
    pub fn new(pgid: Pgid, leader: usize, session: usize) -> Self {
        Self {
            pgid,
            leader,
            members: Mutex::new(vec![leader]),
            session_id: session,
            foreground: AtomicBool::new(false),
        }
    }

    /// 添加成员。
    pub fn add_member(&self, pid: usize) {
        let mut members = self.members.lock().unwrap();
        if !members.contains(&pid) {
            members.push(pid);
        }
    }

    /// 移除成员。
    pub fn remove_member(&self, pid: usize) -> bool {
        let mut members = self.members.lock().unwrap();
        let before = members.len();
        members.retain(|&m| m != pid);
        members.len() < before
    }

    /// 判断是否为空。
    pub fn is_empty(&self) -> bool {
        self.members.lock().unwrap().is_empty()
    }

    /// 返回成员数量。
    pub fn member_count(&self) -> usize {
        self.members.lock().unwrap().len()
    }

    /// 判断是否为组长。
    pub fn is_leader(&self, pid: usize) -> bool {
        self.leader == pid
    }

    /// 设置前台状态。
    pub fn set_foreground(&self, fg: bool) {
        self.foreground.store(fg, Ordering::Relaxed);
    }

    /// 判断是否前台。
    pub fn is_foreground(&self) -> bool {
        self.foreground.load(Ordering::Relaxed)
    }

    /// 广播信号。
    pub fn broadcast_signal(&self, signo: i32, tasks: &TaskTable) {
        let member_ids = {
            let members = self.members.lock().unwrap();
            members.clone()
        };
        for pid in member_ids {
            let task = tasks.find(pid);
            if let Some(t) = task {
                t.send_sig(signo, self.leader as isize);
            }
        }
    }
}

/// 通用等待队列。
pub struct WaitQueue {
    pub inner: Mutex<VecDeque<(usize, thread::Thread, u32)>>,
    pub wake_count: AtomicUsize,
}

impl WaitQueue {
    /// 构造新的实例。
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            wake_count: AtomicUsize::new(0),
        }
    }

    /// 睡眠等待。
    pub fn sleep(&self, key: usize, flags: u32) {
        let mut q = self.inner.lock().unwrap();
        q.push_back((key, thread::current(), flags));
        drop(q);
        thread::park();
    }

    /// 带超时睡眠。
    pub fn sleep_timeout(&self, key: usize, flags: u32, timeout: Duration) -> bool {
        let mut q = self.inner.lock().unwrap();
        q.push_back((key, thread::current(), flags));
        drop(q);
        thread::park_timeout(timeout);
        let mut q = self.inner.lock().unwrap();
        let before = q.len();
        q.retain(|(k, _, _)| *k != key);
        q.len() < before
    }

    /// 唤醒一个。
    pub fn wake_one(&self, key: usize) -> bool {
        let mut q = self.inner.lock().unwrap();
        if let Some(pos) = q.iter().position(|(k, _, _)| *k == key) {
            let (_, thread, _) = q.remove(pos).unwrap();
            thread.unpark();
            self.wake_count.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// 唤醒所有。
    pub fn wake_all(&self, key: usize) -> usize {
        let mut q = self.inner.lock().unwrap();
        let mut count = 0;
        let mut remaining = VecDeque::new();
        for entry in q.drain(..) {
            if entry.0 == key {
                entry.1.unpark();
                count += 1;
            } else {
                remaining.push_back(entry);
            }
        }
        *q = remaining;
        self.wake_count.fetch_add(count, Ordering::Relaxed);
        count
    }

    /// 按条件唤醒。
    pub fn wake_filtered(&self, pred: impl Fn(usize, u32) -> bool) -> usize {
        let mut q = self.inner.lock().unwrap();
        let mut count = 0;
        let mut remaining = VecDeque::new();
        for entry in q.drain(..) {
            if pred(entry.0, entry.2) {
                entry.1.unpark();
                count += 1;
            } else {
                remaining.push_back(entry);
            }
        }
        *q = remaining;
        self.wake_count.fetch_add(count, Ordering::Relaxed);
        count
    }

    /// 返回等待者数量。
    pub fn pending_count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// 返回总唤醒次数。
    pub fn total_wakes(&self) -> usize {
        self.wake_count.load(Ordering::Relaxed)
    }

    /// 判断是否有等待者。
    pub fn has_waiters_for(&self, key: usize) -> bool {
        self.inner.lock().unwrap().iter().any(|(k, _, _)| *k == key)
    }

    /// 按优先级重排。
    pub fn reorder_by_priority(&self) {
        let mut q = self.inner.lock().unwrap();
        let mut vec: Vec<_> = q.drain(..).collect();
        vec.sort_by(|a, b| a.2.cmp(&b.2));
        q.extend(vec);
    }
}

/// 进程资源限制。
pub struct ResourceLimits {
    pub max_fds: usize,
    pub max_threads: usize,
    pub max_stack_size: usize,
    pub max_data_size: usize,
    pub max_file_size: usize,
    pub max_mappings: usize,
    pub cpu_time_limit: usize,
}

impl ResourceLimits {
    /// 返回默认限制。
    pub fn default_limits() -> Self {
        Self {
            max_fds: 1024,
            max_threads: 256,
            max_stack_size: USR_STK_SZ * 4,
            max_data_size: KHEAP_SZ,
            max_file_size: usize::MAX,
            max_mappings: 65536,
            cpu_time_limit: 0,
        }
    }

    /// 检查 fd 是否超限。
    pub fn check_fd(&self, current: usize) -> bool {
        current < self.max_fds
    }
    /// 检查线程数是否超限。
    pub fn check_threads(&self, current: usize) -> bool {
        current < self.max_threads
    }
    /// 检查栈是否超限。
    pub fn check_stack(&self, requested: usize) -> bool {
        requested <= self.max_stack_size
    }
    /// 检查数据段是否超限。
    pub fn check_data(&self, requested: usize) -> bool {
        requested <= self.max_data_size
    }
    /// 检查文件大小是否超限。
    pub fn check_filesize(&self, requested: usize) -> bool {
        requested <= self.max_file_size
    }
    /// 检查映射数量是否超限。
    pub fn check_mappings(&self, current: usize) -> bool {
        current < self.max_mappings
    }

    /// 继承限制。
    pub fn inherit(&self) -> Self {
        Self {
            max_fds: self.max_fds,
            max_threads: self.max_threads,
            max_stack_size: self.max_stack_size,
            max_data_size: self.max_data_size,
            max_file_size: self.max_file_size,
            max_mappings: self.max_mappings,
            cpu_time_limit: self.cpu_time_limit,
        }
    }

    /// 设置资源限制。
    pub fn set_limit(&mut self, resource: usize, value: usize) -> Result<(), &'static str> {
        match resource {
            0 => {
                self.cpu_time_limit = value;
                Ok(())
            }
            1 => {
                self.max_file_size = value;
                Ok(())
            }
            2 => {
                self.max_data_size = value;
                Ok(())
            }
            3 => {
                self.max_stack_size = value;
                Ok(())
            }
            7 => {
                self.max_fds = value;
                Ok(())
            }
            _ => Err("einval"),
        }
    }

    /// 获取资源限制。
    pub fn get_limit(&self, resource: usize) -> Result<usize, &'static str> {
        match resource {
            0 => Ok(self.cpu_time_limit),
            1 => Ok(self.max_file_size),
            2 => Ok(self.max_data_size),
            3 => Ok(self.max_stack_size),
            7 => Ok(self.max_fds),
            _ => Err("einval"),
        }
    }

    /// 检查是否任一资源超限。
    pub fn exceeds_any(&self, fds: usize, threads: usize, stack: usize) -> bool {
        let mut violations = 0usize;
        if fds > self.max_fds {
            violations += 1;
        }
        if threads > self.max_threads {
            violations += 1;
        }
        if stack > self.max_stack_size {
            violations += 1;
        }
        violations > 0
    }
}

/// 按位合并。
pub fn bitwise_merge(a: u64, b: u64, mask: u64) -> u64 {
    (a & !mask) | (b & mask)
}

/// 旋转位。
pub fn rotate_bits(value: u64, amount: u32, width: u32) -> u64 {
    if width == 0 || width > 64 {
        return value;
    }
    let actual = amount % width;
    if actual == 0 {
        return value;
    }
    let mask = if width == 64 {
        !0u64
    } else {
        (1u64 << width) - 1
    };
    let v = value & mask;
    ((v << actual) | (v >> (width - actual))) & mask
}

/// 64 位 popcount。
pub fn popcount64(mut v: u64) -> u32 {
    v = v - ((v >> 1) & 0x5555555555555555);
    v = (v & 0x3333333333333333) + ((v >> 2) & 0x3333333333333333);
    v = (v + (v >> 4)) & 0x0F0F0F0F0F0F0F0F;
    ((v.wrapping_mul(0x0101010101010101)) >> 56) as u32
}

/// 64 位前导零计数。
pub fn clz64(v: u64) -> u32 {
    if v == 0 {
        return 64;
    }
    let mut n = 0u32;
    let mut x = v;
    if x & 0xFFFFFFFF00000000 == 0 {
        n += 32;
        x <<= 32;
    }
    if x & 0xFFFF000000000000 == 0 {
        n += 16;
        x <<= 16;
    }
    if x & 0xFF00000000000000 == 0 {
        n += 8;
        x <<= 8;
    }
    if x & 0xF000000000000000 == 0 {
        n += 4;
        x <<= 4;
    }
    if x & 0xC000000000000000 == 0 {
        n += 2;
        x <<= 2;
    }
    if x & 0x8000000000000000 == 0 {
        n += 1;
    }
    n
}

/// 64 位第一个置位。
pub fn ffs64(v: u64) -> Option<u32> {
    if v == 0 {
        return None;
    }
    Some(63 - clz64(v & v.wrapping_neg()))
}

/// 向上对齐。
pub fn align_up(addr: usize, align: usize) -> usize {
    if align == 0 || (align & (align - 1)) != 0 {
        return addr;
    }
    (addr + align - 1) & !(align - 1)
}

/// 向下对齐。
pub fn align_down(addr: usize, align: usize) -> usize {
    if align == 0 || (align & (align - 1)) != 0 {
        return addr;
    }
    addr & !(align - 1)
}

/// 判断是否为 2 的幂。
pub fn is_power_of_two(v: usize) -> bool {
    v != 0 && (v & (v - 1)) == 0
}

/// 向下取整 log2。
pub fn log2_floor(v: usize) -> usize {
    if v == 0 {
        return 0;
    }
    (std::mem::size_of::<usize>() * 8) - 1 - (v.leading_zeros() as usize)
}

/// 根据页数计算 order。
pub fn order_for_pages(pages: usize) -> usize {
    if pages <= 1 {
        return 0;
    }
    let mut order = 0;
    let mut block = 1;
    while block < pages {
        block <<= 1;
        order += 1;
    }
    order
}

/// 组合哈希。
pub fn hash_combine(seed: u64, value: u64) -> u64 {
    seed ^ (value
        .wrapping_mul(0x9e3779b97f4a7c15)
        .wrapping_add(seed << 6)
        .wrapping_add(seed >> 2))
}

/// murmurhash3 终结。
pub fn murmurhash3_finalize(mut h: u64) -> u64 {
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51afd7ed558ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ceb9fe1a85ec53);
    h ^= h >> 33;
    h
}

// BuddyAllocator 的相关统计，用于性能评估
#[derive(Clone, Copy, Default)]
/// 伙伴分配器统计。
pub struct BuddyStatistics {
    pub alloc_count: usize,
    pub free_count: usize,
    pub split_count: usize,
    pub merge_count: usize,
    pub exact_hit_count: usize,
    pub failed_alloc_count: usize,
}

/// 伙伴分配器。
pub struct BuddyAllocator {
    pub free_lists: Vec<Vec<usize>>,
    pub max_order: usize,
    pub base_addr: usize,
    pub total_pages: usize,
    pub allocated: AtomicUsize,
    pub addr_order_map: BTreeMap<usize, usize>, // 记录 addr -> order，防止传入错误的 order 信息
    pub statistics: BuddyStatistics,            // 统计信息
}

impl BuddyAllocator {
    /// 构造新的实例。
    pub fn new(base: usize, total_pages: usize, max_order: usize) -> Self {
        let mut free_lists = Vec::with_capacity(max_order + 1);
        for _ in 0..=max_order {
            free_lists.push(Vec::new());
        }
        let order = log2_floor(total_pages);
        let usable_order = min(order, max_order);
        let block_pages = 1 << usable_order;
        let mut addr = base;
        let mut remaining = total_pages;
        while remaining >= block_pages {
            free_lists[usable_order].push(addr);
            addr += block_pages * PAGE_SZ;
            remaining -= block_pages;
        }
        for o in (0..usable_order).rev() {
            let pages = 1 << o;
            while remaining >= pages {
                free_lists[o].push(addr);
                addr += pages * PAGE_SZ;
                remaining -= pages;
            }
        }
        Self {
            free_lists,
            max_order,
            base_addr: base,
            total_pages,
            allocated: AtomicUsize::new(0),
            addr_order_map: BTreeMap::new(),
            statistics: BuddyStatistics::default(),
        }
    }

    /// 分配指定 order。
    pub fn alloc_order(&mut self, order: usize) -> Option<usize> {
        self.alloc_order_aligned(order, 1)
    }

    /// 按对齐分配指定 order。
    pub fn alloc_order_aligned(&mut self, order: usize, align: usize) -> Option<usize> {
        if order > self.max_order {
            self.statistics.failed_alloc_count += 1;
            return None;
        }

        let align_pages = if align < 1 { 1 } else { align };
        let target_pages = 1usize << order;
        let mut found = None;

        // allocate
        for o in order..=self.max_order {
            let block_pages = 1usize << o;
            for (pos, &block) in self.free_lists[o].iter().enumerate() {
                let start_frame = (block - self.base_addr) / PAGE_SZ;
                let end_frame = start_frame + block_pages;
                let mut frame = start_frame;
                while frame + target_pages <= end_frame {
                    if frame % align_pages == 0 {
                        let target_addr = self.base_addr + frame * PAGE_SZ;
                        found = Some((o, pos, block, target_addr));
                        break;
                    }
                    frame += target_pages;
                }
                if found.is_some() {
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }

        let (source_order, pos, block, target_addr) = match found {
            Some(v) => v,
            None => {
                self.statistics.failed_alloc_count += 1;
                return None;
            }
        };

        self.free_lists[source_order].remove(pos);

        // split
        let mut current_order = source_order;
        let mut addr = block;
        while current_order > order {
            current_order -= 1;
            let half_size = (1usize << current_order) * PAGE_SZ;
            let right = addr + half_size;
            if target_addr < right {
                self.free_lists[current_order].push(right);
            } else {
                self.free_lists[current_order].push(addr);
                addr = right;
            }
            self.statistics.split_count += 1;
        }

        self.allocated.fetch_add(1 << order, Ordering::Relaxed);
        self.statistics.alloc_count += 1;
        if source_order == order {
            self.statistics.exact_hit_count += 1;
        }
        self.addr_order_map.insert(addr, order);
        Some(addr)
    }

    /// 分配单页。
    pub fn alloc_page(&mut self) -> Option<usize> {
        // 单页
        self.alloc_order(0)
    }

    /// 分配多页。
    pub fn alloc_pages(&mut self, pages: usize) -> Option<usize> {
        // 连续
        if pages == 0 {
            return None;
        }
        let order = order_for_pages(pages);
        self.alloc_order(order)
    }

    /// 释放。
    pub fn free(&mut self, addr: usize) {
        if addr % PAGE_SZ != 0 {
            return;
        }

        let real_order = match self.addr_order_map.remove(&addr) {
            Some(o) => o,
            None => return,
        };

        let end = self.base_addr + self.total_pages * PAGE_SZ;
        let block_bytes = (1usize << real_order) * PAGE_SZ;
        let block_end = addr + block_bytes;
        if addr < self.base_addr || block_end > end || (addr - self.base_addr) % block_bytes != 0 {
            self.addr_order_map.insert(addr, real_order); // 越界时 restore
            return;
        }

        let mut current_addr = addr;
        let mut current_order = real_order;
        while current_order < self.max_order {
            let block_size = (1usize << current_order) * PAGE_SZ;
            let rel = current_addr - self.base_addr;
            let buddy_addr = self.base_addr + (rel ^ block_size);
            if let Some(pos) = self.free_lists[current_order]
                .iter()
                .position(|&a| a == buddy_addr)
            {
                self.free_lists[current_order].remove(pos);
                current_addr = min(current_addr, buddy_addr);
                current_order += 1;
                self.statistics.merge_count += 1;
            } else {
                break;
            }
        }
        self.free_lists[current_order].push(current_addr);
        self.allocated.fetch_sub(1 << real_order, Ordering::Relaxed);
        self.statistics.free_count += 1;
    }

    /// 返回空闲页数。
    pub fn free_pages_count(&self) -> usize {
        let mut count = 0;
        for (order, list) in self.free_lists.iter().enumerate() {
            count += list.len() * (1 << order);
        }
        count
    }

    /// 判断地址是否空闲。
    pub fn is_free_addr(&self, addr: usize) -> bool {
        if addr < self.base_addr || addr >= self.base_addr + self.total_pages * PAGE_SZ {
            return false;
        }
        if addr % PAGE_SZ != 0 {
            return false;
        }
        for (order, list) in self.free_lists.iter().enumerate() {
            let block_size = (1usize << order) * PAGE_SZ;
            for &block in list {
                if addr >= block && addr < block + block_size {
                    return true;
                }
            }
        }
        false
    }

    /// 返回最大空闲 order。
    pub fn largest_free_order(&self) -> usize {
        for o in (0..=self.max_order).rev() {
            if !self.free_lists[o].is_empty() {
                return o;
            }
        }
        0
    }

    /// 返回碎片评分。
    pub fn fragmentation_score(&self) -> usize {
        let total_free = self.free_pages_count();
        if total_free == 0 {
            return 0;
        }
        let largest = self.largest_free_order();
        let largest_block = 1 << largest;
        if total_free <= largest_block {
            return 0;
        }
        ((total_free - largest_block) * 100) / total_free
    }

    /// 返回当前状态快照。
    pub fn snapshot(&self) -> BuddyAllocator {
        BuddyAllocator {
            free_lists: self.free_lists.clone(),
            max_order: self.max_order,
            base_addr: self.base_addr,
            total_pages: self.total_pages,
            allocated: AtomicUsize::new(self.allocated.load(Ordering::Relaxed)),
            addr_order_map: self.addr_order_map.clone(),
            statistics: self.statistics,
        }
    }
}

#[derive(Clone, Copy)]
/// 启发式策略参数。
pub struct HeuristicPolicy {
    pub order0_base_target: usize,
    pub order1_base_target: usize,
    pub order2_base_target: usize,
    pub order3_base_target: usize,
    pub order0_max_target: usize,
    pub order1_max_target: usize,
    pub order2_max_target: usize,
    pub order3_max_target: usize,
    pub high_order_target: usize,
    pub protect_min_order: usize,
    pub feedback_window_ops: usize,
    pub feedback_pool_fraction: usize,
    pub pressure_decay_divisor: usize,
}

impl Default for HeuristicPolicy {
    /// 返回默认实例。
    fn default() -> Self {
        Self {
            order0_base_target: 4,
            order1_base_target: 2,
            order2_base_target: 1,
            order3_base_target: 1,
            order0_max_target: 32,
            order1_max_target: 16,
            order2_max_target: 8,
            order3_max_target: 4,
            high_order_target: 1,
            protect_min_order: 4,
            feedback_window_ops: 128,
            feedback_pool_fraction: 16,
            pressure_decay_divisor: 2,
        }
    }
}

#[derive(Clone, Copy, Default)]
/// 启发式分配器统计。
pub struct HeuristicStatistics {
    pub free_list_alloc_count: usize,
    pub preserve_count: usize,
    pub active_coalesce_count: usize,
    pub active_coalesce_pass_count: usize,
    pub pressure_enter_count: usize,
    pub pressure_exit_count: usize,
    pub feedback_update_count: usize,
    pub pressure_decay_count: usize,
}

/// 启发式页帧分配器。
pub struct HeuristicAllocator {
    pub free_lists: Vec<Vec<usize>>,
    pub max_order: usize,
    pub base_addr: usize,
    pub total_pages: usize,
    pub allocated: AtomicUsize,
    pub addr_order_map: BTreeMap<usize, usize>,
    pub statistics: BuddyStatistics,
    pub policy: HeuristicPolicy,
    pub heuristic_statistics: HeuristicStatistics,
    pub dynamic_targets: [usize; 4],
    pub recent_allocs: [usize; 4],
    pub feedback_ops: usize,
    pub merge_pressure: bool,
}

impl HeuristicAllocator {
    const SPLIT_COST_PER_ORDER: isize = 100;
    const REFILL_BONUS_PER_ORDER: isize = 35;
    const RESERVE_DEFICIT_PENALTY: isize = 30;
    const MERGEABLE_EXACT_PENALTY: isize = 40;
    const ISOLATED_EXACT_BONUS: isize = -20;

    /// 构造新的实例。
    pub fn new(base: usize, total_pages: usize, max_order: usize) -> Self {
        Self::with_policy(base, total_pages, max_order, HeuristicPolicy::default())
    }

    /// 携带策略构造。
    pub fn with_policy(
        base: usize,
        total_pages: usize,
        max_order: usize,
        policy: HeuristicPolicy,
    ) -> Self {
        let mut free_lists = Vec::with_capacity(max_order + 1);
        for _ in 0..=max_order {
            free_lists.push(Vec::new());
        }
        let order = log2_floor(total_pages);
        let usable_order = min(order, max_order);
        let block_pages = 1 << usable_order;
        let mut addr = base;
        let mut remaining = total_pages;
        while remaining >= block_pages {
            free_lists[usable_order].push(addr);
            addr += block_pages * PAGE_SZ;
            remaining -= block_pages;
        }
        for o in (0..usable_order).rev() {
            let pages = 1 << o;
            while remaining >= pages {
                free_lists[o].push(addr);
                addr += pages * PAGE_SZ;
                remaining -= pages;
            }
        }
        let dynamic_targets = Self::scaled_targets_for_pool(
            total_pages,
            policy.feedback_pool_fraction,
            Self::policy_base_targets(&policy),
        );
        Self {
            free_lists,
            max_order,
            base_addr: base,
            total_pages,
            allocated: AtomicUsize::new(0),
            addr_order_map: BTreeMap::new(),
            statistics: BuddyStatistics::default(),
            policy,
            heuristic_statistics: HeuristicStatistics::default(),
            dynamic_targets,
            recent_allocs: [0; 4],
            feedback_ops: 0,
            merge_pressure: false,
        }
    }

    /// 分配指定 order。
    pub fn alloc_order(&mut self, order: usize) -> Option<usize> {
        self.alloc_order_aligned(order, 1)
    }

    /// 按对齐分配指定 order。
    pub fn alloc_order_aligned(&mut self, order: usize, align: usize) -> Option<usize> {
        self.record_alloc_request(order);

        if order > self.max_order {
            self.statistics.failed_alloc_count += 1;
            return None;
        }

        let align_pages = if align < 1 { 1 } else { align };

        let target_pages = 1usize << order;

        loop {
            if let Some(addr) = self.try_fast_exact_alloc(order, align_pages) {
                return Some(addr);
            }

            let mut found = None;
            let mut best_score = isize::MAX;

            for o in order..=self.max_order {
                let block_pages = 1usize << o;
                for (pos, &block) in self.free_lists[o].iter().enumerate() {
                    let start_frame = (block - self.base_addr) / PAGE_SZ;
                    let end_frame = start_frame + block_pages;
                    let mut frame = start_frame;
                    while frame + target_pages <= end_frame {
                        if frame % align_pages == 0 {
                            let target_addr = self.base_addr + frame * PAGE_SZ;
                            let score = self.alloc_candidate_score(order, o, block, target_addr);
                            if score < best_score {
                                best_score = score;
                                found = Some((o, pos, block, target_addr));
                            }
                            break;
                        }
                        frame += target_pages;
                    }
                }

                if found.is_some() && o < self.max_order {
                    let next_order = o + 1;
                    if best_score <= self.min_possible_alloc_score(order, next_order) {
                        break;
                    }
                }
            }

            let (source_order, pos, block, target_addr) = match found {
                Some(v) => v,
                None => {
                    if order >= self.policy.protect_min_order {
                        self.enter_merge_pressure();
                        if self.coalesce_free_lists_for_pressure() {
                            continue;
                        }
                    }
                    self.statistics.failed_alloc_count += 1;
                    return None;
                }
            };

            return Some(self.alloc_from_free_list(order, source_order, pos, block, target_addr));
        }
    }

    /// 分配单页。
    pub fn alloc_page(&mut self) -> Option<usize> {
        self.alloc_order(0)
    }

    /// 分配多页。
    pub fn alloc_pages(&mut self, pages: usize) -> Option<usize> {
        if pages == 0 {
            return None;
        }
        let order = order_for_pages(pages);
        self.alloc_order(order)
    }

    /// 释放。
    pub fn free(&mut self, addr: usize) {
        if addr % PAGE_SZ != 0 {
            return;
        }

        let real_order = match self.addr_order_map.remove(&addr) {
            Some(o) => o,
            None => return,
        };

        let end = self.base_addr + self.total_pages * PAGE_SZ;
        let block_bytes = (1usize << real_order) * PAGE_SZ;
        let block_end = addr + block_bytes;
        if addr < self.base_addr || block_end > end || (addr - self.base_addr) % block_bytes != 0 {
            self.addr_order_map.insert(addr, real_order);
            return;
        }

        if self.free_to_buddy(addr, real_order, true) {
            self.leave_merge_pressure();
        } else {
            self.addr_order_map.insert(addr, real_order);
        }
    }

    /// 返回空闲页数。
    pub fn free_pages_count(&self) -> usize {
        let mut count = 0;
        for (order, list) in self.free_lists.iter().enumerate() {
            count += list.len() * (1 << order);
        }
        count
    }

    /// 判断地址是否空闲。
    pub fn is_free_addr(&self, addr: usize) -> bool {
        if addr < self.base_addr || addr >= self.base_addr + self.total_pages * PAGE_SZ {
            return false;
        }
        if addr % PAGE_SZ != 0 {
            return false;
        }
        for (order, list) in self.free_lists.iter().enumerate() {
            let block_size = (1usize << order) * PAGE_SZ;
            for &block in list {
                if addr >= block && addr < block + block_size {
                    return true;
                }
            }
        }
        false
    }

    /// 返回最大空闲 order。
    pub fn largest_free_order(&self) -> usize {
        for o in (0..=self.max_order).rev() {
            if !self.free_lists[o].is_empty() {
                return o;
            }
        }
        0
    }

    /// 返回碎片评分。
    pub fn fragmentation_score(&self) -> usize {
        let total_free = self.free_pages_count();
        if total_free == 0 {
            return 0;
        }
        let largest = self.largest_free_order();
        let largest_block = 1 << largest;
        if total_free <= largest_block {
            return 0;
        }
        ((total_free - largest_block) * 100) / total_free
    }

    /// 分配器统计。
    pub fn allocator_statistics(&self) -> BuddyStatistics {
        self.statistics
    }

    /// 启发式统计。
    pub fn heuristic_statistics(&self) -> HeuristicStatistics {
        self.heuristic_statistics
    }

    /// 返回当前状态快照。
    pub fn snapshot(&self) -> HeuristicAllocator {
        HeuristicAllocator {
            free_lists: self.free_lists.clone(),
            max_order: self.max_order,
            base_addr: self.base_addr,
            total_pages: self.total_pages,
            allocated: AtomicUsize::new(self.allocated.load(Ordering::Relaxed)),
            addr_order_map: self.addr_order_map.clone(),
            statistics: self.statistics,
            policy: self.policy,
            heuristic_statistics: self.heuristic_statistics,
            dynamic_targets: self.dynamic_targets,
            recent_allocs: self.recent_allocs,
            feedback_ops: self.feedback_ops,
            merge_pressure: self.merge_pressure,
        }
    }

    /// 空闲to伙伴。
    fn free_to_buddy(&mut self, addr: usize, real_order: usize, count_as_free: bool) -> bool {
        // count_as_free == true: 普通 free，需要 allocated -= pages, free_count += 1
        // count_as_free == false: 内部整理 free block，不再改 allocated/free_count

        if real_order > self.max_order || addr % PAGE_SZ != 0 {
            return false;
        }

        let end = self.base_addr + self.total_pages * PAGE_SZ;
        let block_bytes = (1usize << real_order) * PAGE_SZ;
        let block_end = addr + block_bytes;

        if addr < self.base_addr || block_end > end || (addr - self.base_addr) % block_bytes != 0 {
            return false;
        }

        let mut current_addr = addr;
        let mut current_order = real_order;

        while current_order < self.max_order {
            if self.should_preserve_order_block(current_order) {
                if self
                    .find_free_buddy_pos(current_addr, current_order)
                    .is_some()
                {
                    self.heuristic_statistics.preserve_count += 1;
                }
                break;
            }

            if let Some((buddy_addr, pos)) = self.find_free_buddy_pos(current_addr, current_order) {
                self.free_lists[current_order].remove(pos);
                current_addr = min(current_addr, buddy_addr);
                current_order += 1;
                self.statistics.merge_count += 1;
            } else {
                break;
            }
        }

        self.free_lists[current_order].push(current_addr);

        if count_as_free {
            self.allocated.fetch_sub(1 << real_order, Ordering::Relaxed);
            self.statistics.free_count += 1;
        }

        true
    }

    /// 尝试fast精确分配。
    fn try_fast_exact_alloc(&mut self, order: usize, align_pages: usize) -> Option<usize> {
        if align_pages != 1 || order > self.max_order || self.free_lists[order].is_empty() {
            return None;
        }

        if let Some(pos) = self.free_lists[order]
            .iter()
            .position(|&block| !self.has_free_buddy_at_order(block, order))
        {
            let block = self.free_lists[order][pos];
            return Some(self.alloc_from_free_list(order, order, pos, block, block));
        }

        if self.free_lists[order].len() > self.target_blocks_at_order(order) {
            let pos = self.free_lists[order].len() - 1;
            let block = self.free_lists[order][pos];
            return Some(self.alloc_from_free_list(order, order, pos, block, block));
        }

        None
    }

    /// 分配from空闲列表。
    fn alloc_from_free_list(
        &mut self,
        request_order: usize,
        source_order: usize,
        pos: usize,
        block: usize,
        target_addr: usize,
    ) -> usize {
        self.free_lists[source_order].remove(pos);

        let mut current_order = source_order;
        let mut addr = block;
        while current_order > request_order {
            current_order -= 1;
            let half_size = (1usize << current_order) * PAGE_SZ;
            let right = addr + half_size;
            if target_addr < right {
                self.free_lists[current_order].push(right);
            } else {
                self.free_lists[current_order].push(addr);
                addr = right;
            }
            self.statistics.split_count += 1;
        }

        self.allocated
            .fetch_add(1 << request_order, Ordering::Relaxed);
        self.statistics.alloc_count += 1;
        self.heuristic_statistics.free_list_alloc_count += 1;
        if source_order == request_order {
            self.statistics.exact_hit_count += 1;
        }
        self.addr_order_map.insert(addr, request_order);
        addr
    }

    /// 返回指定 order 的目标。
    fn target_blocks_at_order(&self, order: usize) -> usize {
        if order >= self.policy.protect_min_order {
            return self.policy.high_order_target;
        }

        if order < self.dynamic_targets.len() {
            self.dynamic_targets[order]
        } else {
            0
        }
    }

    /// 返回策略rgets。
    fn policy_base_targets(policy: &HeuristicPolicy) -> [usize; 4] {
        [
            policy.order0_base_target,
            policy.order1_base_target,
            policy.order2_base_target,
            policy.order3_base_target,
        ]
    }

    /// 返回策略rgets。
    fn policy_max_targets(policy: &HeuristicPolicy) -> [usize; 4] {
        let base = Self::policy_base_targets(policy);
        [
            max(policy.order0_max_target, base[0]),
            max(policy.order1_max_target, base[1]),
            max(policy.order2_max_target, base[2]),
            max(policy.order3_max_target, base[3]),
        ]
    }

    /// 返回缩放后的targetsforpool。
    fn scaled_targets_for_pool(
        total_pages: usize,
        feedback_pool_fraction: usize,
        raw_targets: [usize; 4],
    ) -> [usize; 4] {
        if total_pages == 0 {
            return [0; 4];
        }
        if feedback_pool_fraction == 0 {
            return raw_targets;
        }

        let budget_pages = max(1, total_pages / feedback_pool_fraction);
        let raw_pages: usize = raw_targets
            .iter()
            .enumerate()
            .map(|(order, &target)| target * (1usize << order))
            .sum();

        if raw_pages <= budget_pages {
            return raw_targets;
        }

        let mut scaled = [0; 4];
        let mut used_pages = 0;

        for order in 0..scaled.len() {
            let block_pages = 1usize << order;
            let remaining_pages = budget_pages.saturating_sub(used_pages);
            let cap_blocks = remaining_pages / block_pages;
            scaled[order] = min(raw_targets[order], cap_blocks);
            used_pages += scaled[order] * block_pages;
        }

        scaled
    }

    /// clamptargetsto策略andpool。
    fn clamp_targets_to_policy_and_pool(&self, raw_targets: [usize; 4]) -> [usize; 4] {
        let base = Self::policy_base_targets(&self.policy);
        let max_targets = Self::policy_max_targets(&self.policy);
        let mut clamped = [0; 4];

        for order in 0..clamped.len() {
            clamped[order] = min(max(raw_targets[order], base[order]), max_targets[order]);
        }

        Self::scaled_targets_for_pool(
            self.total_pages,
            self.policy.feedback_pool_fraction,
            clamped,
        )
    }

    /// 返回缩放后的基础targets。
    fn scaled_base_targets(&self) -> [usize; 4] {
        Self::scaled_targets_for_pool(
            self.total_pages,
            self.policy.feedback_pool_fraction,
            Self::policy_base_targets(&self.policy),
        )
    }

    /// record分配request。
    fn record_alloc_request(&mut self, order: usize) {
        if order < self.recent_allocs.len() {
            self.recent_allocs[order] = self.recent_allocs[order].saturating_add(1);
        }
        self.feedback_ops = self.feedback_ops.saturating_add(1);
        self.maybe_update_feedback_targets();
    }

    /// maybe更新反馈targets。
    fn maybe_update_feedback_targets(&mut self) {
        if self.policy.feedback_window_ops == 0 {
            return;
        }
        if self.feedback_ops < self.policy.feedback_window_ops {
            return;
        }

        self.recompute_dynamic_targets();
        self.recent_allocs = [0; 4];
        self.feedback_ops = 0;
        self.heuristic_statistics.feedback_update_count += 1;
    }

    /// recomputedynamictargets。
    fn recompute_dynamic_targets(&mut self) {
        let base = Self::policy_base_targets(&self.policy);
        let max_targets = Self::policy_max_targets(&self.policy);
        let total_recent_allocs: usize = self.recent_allocs.iter().sum();
        let mut raw_targets = base;

        if total_recent_allocs > 0 {
            for order in 0..raw_targets.len() {
                let max_bonus = max_targets[order].saturating_sub(base[order]);
                let bonus = self.recent_allocs[order] * max_bonus / total_recent_allocs;
                raw_targets[order] = base[order] + bonus;
            }
        }

        self.dynamic_targets = self.clamp_targets_to_policy_and_pool(raw_targets);
    }

    /// 衰减dynamictargetsfor压力。
    fn decay_dynamic_targets_for_pressure(&mut self) {
        let divisor = self.policy.pressure_decay_divisor;
        if divisor <= 1 {
            return;
        }

        let base = self.scaled_base_targets();
        let mut changed = false;

        for order in 0..self.dynamic_targets.len() {
            let decayed = max(base[order], self.dynamic_targets[order] / divisor);
            if decayed < self.dynamic_targets[order] {
                self.dynamic_targets[order] = decayed;
                changed = true;
            }
        }

        if changed {
            self.heuristic_statistics.pressure_decay_count += 1;
        }
    }

    /// should保留order块。
    fn should_preserve_order_block(&self, order: usize) -> bool {
        if self.merge_pressure {
            return false;
        }

        let target = self.target_blocks_at_order(order);
        target > 0 && self.exact_free_blocks_at_order(order) < target
    }

    /// 伙伴地址。
    fn buddy_addr(&self, addr: usize, order: usize) -> Option<usize> {
        if order >= self.max_order || addr < self.base_addr {
            return None;
        }

        let block_size = (1usize << order) * PAGE_SZ;
        let rel = addr - self.base_addr;
        let buddy_addr = self.base_addr + (rel ^ block_size);
        let end = self.base_addr + self.total_pages * PAGE_SZ;

        if buddy_addr < self.base_addr || buddy_addr + block_size > end {
            return None;
        }

        Some(buddy_addr)
    }

    /// 查找伙伴pos。
    fn find_free_buddy_pos(&self, addr: usize, order: usize) -> Option<(usize, usize)> {
        let buddy_addr = self.buddy_addr(addr, order)?;
        let pos = self.free_lists[order]
            .iter()
            .position(|&a| a == buddy_addr)?;
        Some((buddy_addr, pos))
    }

    /// 判断是否s空闲伙伴atorder。
    fn has_free_buddy_at_order(&self, addr: usize, order: usize) -> bool {
        self.find_free_buddy_pos(addr, order).is_some()
    }

    /// 返回有效der。
    fn effective_free_blocks_at_order(&self, order: usize) -> usize {
        if order > self.max_order {
            return 0;
        }

        let mut count = 0;

        // 高阶块可拆的情况下也算进去
        for source_order in order..=self.max_order {
            count += self.free_lists[source_order].len() * (1usize << (source_order - order));
        }

        count
    }

    /// 返回精确atorder。
    fn exact_free_blocks_at_order(&self, order: usize) -> usize {
        if order > self.max_order {
            return 0;
        }

        self.free_lists[order].len()
    }

    /// 返回精确torder。
    fn exact_deficit_at_order(&self, order: usize) -> usize {
        let target = self.target_blocks_at_order(order);
        target.saturating_sub(self.exact_free_blocks_at_order(order))
    }

    /// 返回精确torder。
    fn exact_surplus_at_order(&self, order: usize) -> usize {
        let target = self.target_blocks_at_order(order);
        self.exact_free_blocks_at_order(order)
            .saturating_sub(target)
    }

    /// 返回第一个ghorderunder目标。
    fn first_high_order_under_target(&self) -> Option<usize> {
        if self.policy.high_order_target == 0 {
            return None;
        }

        for order in self.policy.protect_min_order..=self.max_order {
            if self.effective_free_blocks_at_order(order) < self.target_blocks_at_order(order) {
                return Some(order);
            }
        }

        None
    }

    /// 返回highorderu目标。
    fn high_order_under_target(&self) -> bool {
        self.first_high_order_under_target().is_some()
    }

    /// enter合并压力。
    fn enter_merge_pressure(&mut self) {
        if !self.merge_pressure {
            self.heuristic_statistics.pressure_enter_count += 1;
            self.decay_dynamic_targets_for_pressure();
        }
        self.merge_pressure = true;
    }

    /// leave合并压力。
    fn leave_merge_pressure(&mut self) {
        if self.merge_pressure && !self.high_order_under_target() {
            self.merge_pressure = false;
            self.heuristic_statistics.pressure_exit_count += 1;
        }
    }

    /// 尝试合并orderonce。
    fn try_coalesce_order_once(&mut self, order: usize) -> bool {
        if order >= self.max_order {
            return false;
        }

        let len = self.free_lists[order].len();
        for i in 0..len {
            let addr = self.free_lists[order][i];
            if let Some((buddy_addr, j)) = self.find_free_buddy_pos(addr, order) {
                if i == j {
                    continue;
                }

                let (first, second) = if i > j { (i, j) } else { (j, i) };
                self.free_lists[order].remove(first);
                self.free_lists[order].remove(second);
                self.free_lists[order + 1].push(min(addr, buddy_addr));
                self.statistics.merge_count += 1;
                self.heuristic_statistics.active_coalesce_count += 1;
                return true;
            }
        }

        false
    }

    /// 合并空闲listsfor压力。
    fn coalesce_free_lists_for_pressure(&mut self) -> bool {
        if self.first_high_order_under_target().is_none() {
            return false;
        }

        self.enter_merge_pressure();
        let mut coalesced = false;

        loop {
            if self.first_high_order_under_target().is_none() {
                self.leave_merge_pressure();
                return coalesced;
            }

            let mut coalesced_this_pass = false;
            self.heuristic_statistics.active_coalesce_pass_count += 1;

            for order in 0..self.max_order {
                if self.first_high_order_under_target().is_none() {
                    break;
                }

                if self.try_coalesce_order_once(order) {
                    coalesced = true;
                    coalesced_this_pass = true;
                }
            }

            if !coalesced_this_pass {
                return coalesced;
            }
        }
    }

    /// 计算分配candida评分。
    fn alloc_candidate_score(
        &self,
        request_order: usize,
        source_order: usize,
        candidate_addr: usize,
        target_addr: usize,
    ) -> isize {
        let split_cost = (source_order - request_order) as isize * Self::SPLIT_COST_PER_ORDER;
        let mut refill_bonus = 0isize;

        let source_exact = self.exact_free_blocks_at_order(source_order);
        let projected_source_exact = source_exact.saturating_sub(1);
        let reserve_penalty =
            self.projected_exact_deficit_penalty(source_order, projected_source_exact);

        for leftover_order in request_order..source_order {
            if self.exact_deficit_at_order(leftover_order) > 0 {
                refill_bonus += Self::REFILL_BONUS_PER_ORDER;
            }
        }

        split_cost + reserve_penalty - refill_bonus
            + self.local_alloc_topology_penalty(
                request_order,
                source_order,
                candidate_addr,
                target_addr,
            )
    }

    /// 返回预计alty。
    fn projected_exact_deficit_penalty(&self, order: usize, projected_exact: usize) -> isize {
        let target = self.target_blocks_at_order(order);
        if target == 0 || projected_exact >= target || self.exact_surplus_at_order(order) > 0 {
            return 0;
        }

        Self::RESERVE_DEFICIT_PENALTY
    }

    /// 返回最小能分配评分。
    fn min_possible_alloc_score(&self, request_order: usize, source_order: usize) -> isize {
        if source_order == request_order {
            return Self::ISOLATED_EXACT_BONUS;
        }

        let split_levels = (source_order - request_order) as isize;
        split_levels * (Self::SPLIT_COST_PER_ORDER - Self::REFILL_BONUS_PER_ORDER)
    }

    /// 返回局部扑penalty。
    fn local_alloc_topology_penalty(
        &self,
        request_order: usize,
        source_order: usize,
        candidate_addr: usize,
        target_addr: usize,
    ) -> isize {
        if source_order != request_order || candidate_addr != target_addr {
            return 0;
        }

        if self.has_free_buddy_at_order(candidate_addr, request_order) {
            Self::MERGEABLE_EXACT_PENALTY
        } else {
            Self::ISOLATED_EXACT_BONUS
        }
    }
}
