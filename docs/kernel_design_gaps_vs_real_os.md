# Chaos Kernel 实际修复记录

本文只记录我们在对话过程中已经实际修改过的设计问题。

约定：

- `kernel/src/kernel.rs`：测试导向版本。修复以尽量通过 `chaos-tests` 为目标，接口和测试期望优先。
- `kernel/src/kernel_refactor.rs`：参考设计版本。修复以贴近正常 OS 设计为目标，可以牺牲测试兼容性和原接口。
- 本文不再记录“只讨论过但没有动代码”的问题。

## 1. GKL / `KernLock` 的 owner 语义

涉及文件：

- `kernel/src/kernel.rs`
- `kernel/src/kernel_refactor.rs`

### 原来的设计

`kernel.rs` 的 GKL 使用：

```rust
pub fn enter(&self, id: usize)
```

调用者手动传入 `id`，锁内部用这个 `id` 记录 holder 和递归深度。释放时也没有严格检查“当前释放者是不是持有者”。

这种设计让 GKL 的 owner 依赖调用方自觉传参，而不是来自当前 kernel 执行上下文。

### 存在的问题

真实 OS 中，锁 owner 应该来自当前 CPU、当前 kernel thread 或当前 task，而不是由调用者随便传一个数字。

旧设计的问题是：

- 同一个 host thread 如果用不同 `id` 进入 GKL，递归判断会失真；
- 非持有线程可能错误释放锁；
- 调用点可能直接操作 `GKL.holder`、`GKL.depth`、`GKL.flag` 这类内部字段，绕过锁接口；
- GKL 的测试用 owner 和真实执行上下文混在一起。

### 修改后的设计

`kernel.rs` 采取测试兼容的最小修复：

- 保留 `enter(id)`，兼容 basic 测试里的 `owner()` 断言；
- 新增/使用 `thread_local! KERN_TID` 表示真实 host thread 上的 kernel context；
- 递归进入用 `thread_id` 判断，而不是只信任传入的 `id`；
- `leave()` 检查锁确实被持有，并检查当前 `KERN_TID` 是 owner；
- 原先直接操作 GKL 内部字段的地方改为走 `GKL.enter(id)` / `GKL.leave()`。

`kernel_refactor.rs` 改成更贴近正常 OS 的接口：

```rust
pub fn enter(&self)
pub fn leave(&self)
```

owner 来自：

```rust
current_kernel_context_id()
```

也就是由当前执行上下文决定，而不是由调用者传参。

## 2. `SyncQueue` 的 pending signal

涉及文件：

- `kernel/src/kernel.rs`
- `kernel/src/kernel_refactor.rs`

### 原来的设计

`kernel.rs` 的 `SyncQueue` 有：

```rust
pending_signals: AtomicUsize
```

如果 `signal()` 发生时没有线程正在等待，它不会丢弃这次 signal，而是记录到 `pending_signals`。之后新的 waiter 调用 `park_on()` 时，即使条件仍不满足，也可能消费这次历史 signal 并直接返回。

### 存在的问题

这不符合正常 condition variable / wait queue 语义。

正常 OS 语义是：

```text
wakeup 只唤醒当前已经睡在等待队列里的线程；
如果没人睡，wakeup 不保存；
真正持久化的是共享条件本身，不是 wakeup 动作。
```

`signal-before-wait` 不应该让后来的 waiter 自动通过，除非共享条件已经变为满足。

### 修改后的设计

`kernel.rs` 保留 `pending_signals`，因为 `basic_condvar_signal_before_wait` 测试依赖这个行为。

`kernel_refactor.rs` 删除 pending signal 语义：

- `SyncQueue` 不再保存 `pending_signals`；
- `signal()` 只 `pop_front()` 唤醒一个已经等待的线程；
- 队列为空时 `signal()` 什么也不保存；
- `park_on()` 会重复检查 predicate，未满足就入队并 `park()`。

这更接近真实 OS 的 condvar / wait queue 设计。

## 3. `CapSet::inherit` 的 mask 语义

涉及文件：

- `kernel/src/kernel.rs`
- `kernel/src/kernel_refactor.rs`

### 原来的设计

`CapSet::inherit()` 里使用了 `INHERITABLE_MASK`，但语义不清。我们确认它应该表示“允许继承的 capability 位”。

### 存在的问题

如果把 `INHERITABLE_MASK` 理解反了，就会继承不该继承的 capability，或者丢掉应该继承的 capability。

能力位这种权限结构必须默认保守。继承逻辑如果写反，会造成权限扩大或权限丢失。

### 修改后的设计

`kernel.rs` 和 `kernel_refactor.rs` 都改成按 mask 保留可继承位：

```rust
let filtered_b = parent.bits & INHERITABLE_MASK;
let filtered_e = parent.effective & INHERITABLE_MASK;
```

也就是：

```text
mask 中为 1 的 capability 才允许继承；
mask 中为 0 的 capability 继承时清掉。
```

## 4. `Sema` 从忙等模型改成等待队列模型

涉及文件：

- `kernel/src/kernel.rs`
- `kernel/src/kernel_refactor.rs`

### 原来的设计

`kernel.rs` 的 `Sema` 主要字段是：

```rust
cnt: isize,
pid: usize,
rm: bool,
bus: EvBus,
```

等待方式是：

```rust
pub fn acquire_spin(&self) -> Result<(), &'static str> {
    loop {
        match self.try_acquire()? {
            true => return Ok(()),
            false => thread::yield_now(),
        }
    }
}
```

也就是说，获取不到 semaphore 时并不睡眠，只是反复 `yield_now()`。

`release()` 增加计数并设置 `EvFlag::SEM_ACQ`，但没有真正唤醒一个挂在 semaphore 等待队列上的线程，因为原设计没有 semaphore 自己的等待队列。

### 存在的问题

真实 semaphore 不应该靠忙等获取。

正常语义应是：

```text
acquire:
    cnt > 0  时直接减一并返回；
    cnt == 0 时把当前线程加入等待队列并睡眠。

release:
    如果有等待者，唤醒一个；
    如果没有等待者，增加可用计数。
```

旧设计的问题是：

- 等待线程消耗 CPU；
- `release()` 没有明确的 waiter 唤醒目标；
- `get_ncnt()` 依赖 `EvBus` callback 数量，不能表示真正等待者数量；
- `set_val()` 改变 semaphore 值时也没有正确唤醒等待线程；
- `pid` 需要外部手动 `set_pid(p)`，不符合 System V semaphore 的 `sempid` 语义。

### 修改后的设计

`kernel.rs` 保留测试导向版本。

`kernel_refactor.rs` 改为睡眠等待模型：

```rust
waiters: VecDeque<thread::Thread>
```

核心变化：

- `acquire()` 获取不到资源时，把当前线程加入 `waiters`，然后 `thread::park()`；
- `release()` 增加计数后，从 `waiters` 中唤醒一个线程；
- `remove()` 标记 removed，并唤醒所有等待者；
- `set_val(v)` 在值变为可获取时唤醒最多 `cnt` 个等待者；
- `get_ncnt()` 返回真实等待队列长度；
- `set_pid()` 不再接收外部 pid，而是记录 `current_pid()`；
- `try_acquire()`、`release()`、`set_val()` 等修改状态的位置也自动更新 pid。

`current_pid()` 目前仍是占位：

```rust
fn current_pid() -> usize {
    Pid::INIT
}
```

它表达了方向：pid 应该来自当前任务上下文，而不是调用者手动传入。

## 5. `AddrSpace::cow_pages` 接入 `SharedPage`

涉及文件：

- `kernel/src/kernel.rs`
- `kernel/src/kernel_refactor.rs`

### 原来的设计

原本 `AddrSpace` 的 COW 表是：

```rust
cow_pages: Mutex<BTreeMap<usize, PgFrame>>
```

`fork_from()` 复制父进程的 `PgFrame` 状态到子进程。这样父子地址空间里保存的是不同的 `PgFrame` 对象，而不是共享同一个 COW page 元数据。

代码中也有 `SharedPage`，但没有真正接入 `AddrSpace.cow_pages`。

### 存在的问题

COW 的核心是“多个地址空间共享同一物理页，写入时再复制”。

旧设计的问题是：

- 父子进程没有共享同一个 COW page 对象；
- refcount 可能只是被复制出来的数值，不是真正共享的引用计数；
- COW fault 时无法自然表达“这个 page 还有几个 sharer”；
- `SharedPage` 存在但没有进入主 COW 路径，结构和行为脱节。

### 修改后的设计

为了不影响测试导向版本，我们把这个改动只保留在 `kernel_refactor.rs`。

`kernel.rs` 回到测试兼容设计：

```rust
cow_pages: Mutex<BTreeMap<usize, PgFrame>>
```

`kernel_refactor.rs` 改成：

```rust
cow_pages: Mutex<BTreeMap<usize, Arc<SharedPage>>>
```

并扩展 `SharedPage`：

```rust
pub struct SharedPage {
    pub frame: AtomicUsize,
    pub ref_count: PgFrame,
    pub w: AtomicBool,
    pub pending: AtomicBool,
}
```

关键行为：

- `fork_from()` 对父进程中的 `Arc<SharedPage>` 执行 `ref_up()`，子进程保存 `Arc::clone(page)`；
- `handle_cow_fault()` 如果 `ref_count <= 1`，直接把 page 标记为 private writable；
- 如果 `ref_count > 1`，分配新 frame，旧 page `ref_down()`，当前地址空间换成新的 private writable `SharedPage`；
- `unmap_range()` 删除映射时对 `SharedPage` 做 `ref_down()`；
- `cow_sharers()` 根据 `SharedPage::ref_get()` 统计共享页。

这比原来的 `PgFrame` 值复制更接近真实 COW 的共享元数据模型。

## 6. `PageCache::evict_lru()` 淘汰 dirty page 前写回

涉及文件：

- `kernel/src/kernel_refactor.rs`

### 原来的设计

`PageCache::evict_lru()` 找到第一个 `pin_count == 0` 的 victim 后直接删除：

```rust
self.entries.remove(&id);
self.lru_order.retain(|&x| x != id);
```

它没有检查该页是否是 dirty page。

### 存在的问题

真实 page cache 中，dirty page 表示内容已经在内存中被修改，但还没有写回后端存储。

如果直接淘汰 dirty page，就等价于丢数据。

真实 OS 在回收 dirty page 前通常要：

```text
writeback 成功 -> 清 dirty -> 可以回收；
writeback 失败 -> 保留 page，并向上报告错误或稍后重试。
```

### 修改后的设计

只在 `kernel_refactor.rs` 中加入占位写回接口：

```rust
pub fn write_back(&self, page_id: usize, data: &[u8]) -> Result<(), &'static str> {
    let _ = (page_id, data.len());
    Ok(())
}
```

并让 `evict_lru()` 在淘汰 dirty page 前先调用它：

```rust
if let Some(e) = self.entries.get(&id) {
    if e.dirty && self.write_back(e.page_id, &e.data).is_err() {
        return false;
    }
}
```

修改后的语义是：

```text
clean page：可以直接淘汰；
dirty page：先 write_back；
write_back 失败：不淘汰；
write_back 成功：再移除缓存项。
```

目前 `write_back()` 仍是占位，没有真正接入 `Disk` 或文件系统后端，但淘汰流程已经更接近正常 OS 设计。

## 7. `kernel.rs` 与 `kernel_refactor.rs` 的职责边界

这个不是单独代码点，而是这轮修复中形成的约定。

### 原来的做法

一开始我们有些修复会直接落到 `kernel.rs`，即使这些修复更像“正常 OS 设计”，也可能影响 basic 测试。

例如 COW/`SharedPage` 的接入一度被改到 `kernel.rs`，后来发现这和“`kernel.rs` 以通过测试为目标”的约定冲突。

### 存在的问题

`kernel.rs` 和 `kernel_refactor.rs` 如果目标混在一起，就会出现两类冲突：

- 为了正常 OS 语义改代码，可能破坏 basic 测试；
- 为了测试行为保留特殊语义，又会污染参考设计。

### 修改后的设计

现在明确区分：

```text
kernel.rs：
    测试导向版本；
    保留测试依赖的特殊语义；
    修复以最小改动、通过 basic 测试为目标。

kernel_refactor.rs：
    参考设计版本；
    允许 API 改动；
    优先贴近真实 OS 设计。
```

因此：

- GKL 在 `kernel.rs` 保留 `enter(id)`，在 `kernel_refactor.rs` 改成 `enter()`；
- `SyncQueue` 的 pending signal 在 `kernel.rs` 保留，在 `kernel_refactor.rs` 删除；
- `Sema` 的睡眠等待队列在 `kernel_refactor.rs` 实现；
- `SharedPage` 接入 COW 只保留在 `kernel_refactor.rs`；
- `PageCache` dirty eviction writeback 先在 `kernel_refactor.rs` 做占位实现。
