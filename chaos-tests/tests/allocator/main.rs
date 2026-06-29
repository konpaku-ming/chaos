// created by claude
use chaos_tests::*;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const BASE: usize = 0x1000_0000;

fn pages(n: usize) -> usize {
    n * PAGE_SZ
}

#[derive(Clone, Copy)]
enum BenchmarkStep {
    AllocOrder(usize),
    AllocAligned { order: usize, align: usize },
    FreeOldest,
    FreeNewest,
    FreeAll,
}

struct BenchmarkReport {
    name: &'static str,
    attempted_ops: usize,
    elapsed: Duration,
    statistics: BuddyStatistics,
    free_pages: usize,
    largest_free_order: usize,
    fragmentation_score: usize,
    live_blocks: usize,
}

fn run_buddy_benchmark(
    name: &'static str,
    total_pages: usize,
    max_order: usize,
    steps: &[BenchmarkStep],
) -> BenchmarkReport {
    let mut alloc = BuddyAllocator::new(BASE, total_pages, max_order);
    let mut live = VecDeque::new();
    let mut attempted_ops = 0;
    let start = Instant::now();

    for step in steps {
        match *step {
            BenchmarkStep::AllocOrder(order) => {
                attempted_ops += 1;
                if let Some(addr) = alloc.alloc_order(order) {
                    live.push_back(addr);
                }
            }
            BenchmarkStep::AllocAligned { order, align } => {
                attempted_ops += 1;
                if let Some(addr) = alloc.alloc_order_aligned(order, align) {
                    live.push_back(addr);
                }
            }
            BenchmarkStep::FreeOldest => {
                if let Some(addr) = live.pop_front() {
                    attempted_ops += 1;
                    alloc.free(addr);
                }
            }
            BenchmarkStep::FreeNewest => {
                if let Some(addr) = live.pop_back() {
                    attempted_ops += 1;
                    alloc.free(addr);
                }
            }
            BenchmarkStep::FreeAll => {
                while let Some(addr) = live.pop_back() {
                    attempted_ops += 1;
                    alloc.free(addr);
                }
            }
        }
    }

    BenchmarkReport {
        name,
        attempted_ops,
        elapsed: start.elapsed(),
        statistics: alloc.statistics,
        free_pages: alloc.free_pages_count(),
        largest_free_order: alloc.largest_free_order(),
        fragmentation_score: alloc.fragmentation_score(),
        live_blocks: live.len(),
    }
}

fn log_benchmark(report: &BenchmarkReport) {
    eprintln!(
        "{}: ops={} elapsed={:?} alloc={} free={} split={} merge={} exact={} failed={} free_pages={} largest_order={} frag={} live={}",
        report.name,
        report.attempted_ops,
        report.elapsed,
        report.statistics.alloc_count,
        report.statistics.free_count,
        report.statistics.split_count,
        report.statistics.merge_count,
        report.statistics.exact_hit_count,
        report.statistics.failed_alloc_count,
        report.free_pages,
        report.largest_free_order,
        report.fragmentation_score,
        report.live_blocks,
    );
}

#[test]
fn buddy_new_empty_pool() {
    let alloc = BuddyAllocator::new(BASE, 0, 3);

    assert_eq!(alloc.free_lists.len(), 4);
    assert_eq!(alloc.free_pages_count(), 0);
    assert_eq!(alloc.largest_free_order(), 0);
    assert_eq!(alloc.fragmentation_score(), 0);
    assert_eq!(alloc.allocated.load(Ordering::Relaxed), 0);
    assert!(alloc.addr_order_map.is_empty());
}

#[test]
fn buddy_new_splits_non_power_of_two_pool() {
    let alloc = BuddyAllocator::new(BASE, 13, 3);

    assert_eq!(alloc.free_pages_count(), 13);
    assert_eq!(alloc.largest_free_order(), 3);
    assert_eq!(alloc.free_lists[3], vec![BASE]);
    assert_eq!(alloc.free_lists[2], vec![BASE + pages(8)]);
    assert!(alloc.free_lists[1].is_empty());
    assert_eq!(alloc.free_lists[0], vec![BASE + pages(12)]);
    assert_eq!(alloc.fragmentation_score(), 38);
}

#[test]
fn buddy_new_respects_max_order() {
    let alloc = BuddyAllocator::new(BASE, 16, 2);

    assert_eq!(alloc.free_pages_count(), 16);
    assert_eq!(alloc.largest_free_order(), 2);
    assert_eq!(
        alloc.free_lists[2],
        vec![BASE, BASE + pages(4), BASE + pages(8), BASE + pages(12)]
    );
}

#[test]
fn buddy_alloc_exact_order_without_split() {
    let mut alloc = BuddyAllocator::new(BASE, 4, 2);

    let addr = alloc.alloc_order(2);

    assert_eq!(addr, Some(BASE));
    assert_eq!(alloc.free_pages_count(), 0);
    assert_eq!(alloc.largest_free_order(), 0);
    assert_eq!(alloc.fragmentation_score(), 0);
    assert_eq!(alloc.allocated.load(Ordering::Relaxed), 4);
    assert_eq!(alloc.addr_order_map.get(&BASE), Some(&2));
    assert_eq!(alloc.statistics.alloc_count, 1);
    assert_eq!(alloc.statistics.exact_hit_count, 1);
    assert_eq!(alloc.statistics.split_count, 0);
    assert_eq!(alloc.statistics.failed_alloc_count, 0);
}

#[test]
fn buddy_alloc_smaller_order_splits_larger_block() {
    let mut alloc = BuddyAllocator::new(BASE, 8, 3);

    let first = alloc.alloc_order(0);
    let second = alloc.alloc_order(0);

    assert_eq!(first, Some(BASE));
    assert_eq!(second, Some(BASE + pages(1)));
    assert_eq!(alloc.free_lists[0], Vec::<usize>::new());
    assert_eq!(alloc.free_lists[1], vec![BASE + pages(2)]);
    assert_eq!(alloc.free_lists[2], vec![BASE + pages(4)]);
    assert_eq!(alloc.free_pages_count(), 6);
    assert_eq!(alloc.largest_free_order(), 2);
    assert_eq!(alloc.fragmentation_score(), 33);
    assert_eq!(alloc.allocated.load(Ordering::Relaxed), 2);
    assert_eq!(alloc.addr_order_map.get(&BASE), Some(&0));
    assert_eq!(alloc.addr_order_map.get(&(BASE + pages(1))), Some(&0));
    assert_eq!(alloc.statistics.alloc_count, 2);
    assert_eq!(alloc.statistics.exact_hit_count, 1);
    assert_eq!(alloc.statistics.split_count, 3);
}

#[test]
fn buddy_alloc_failure_updates_statistics() {
    let mut alloc = BuddyAllocator::new(BASE, 2, 1);

    assert_eq!(alloc.alloc_order(2), None);
    assert_eq!(alloc.statistics.failed_alloc_count, 1);

    assert_eq!(alloc.alloc_order(1), Some(BASE));
    assert_eq!(alloc.alloc_order(1), None);

    assert_eq!(alloc.free_pages_count(), 0);
    assert_eq!(alloc.allocated.load(Ordering::Relaxed), 2);
    assert_eq!(alloc.statistics.alloc_count, 1);
    assert_eq!(alloc.statistics.failed_alloc_count, 2);
}

#[test]
fn buddy_page_helpers_wrap_order_allocator() {
    let mut alloc = BuddyAllocator::new(BASE, 4, 2);

    let first = alloc.alloc_page();
    let second = alloc.alloc_pages(3);

    assert_eq!(first, Some(BASE));
    assert_eq!(second, None);
    assert_eq!(alloc.free_pages_count(), 3);
    assert_eq!(alloc.addr_order_map.get(&BASE), Some(&0));

    alloc.free(BASE);

    assert_eq!(alloc.free_pages_count(), 4);
    assert!(alloc.addr_order_map.is_empty());
}

#[test]
fn buddy_alloc_pages_rounds_up_to_buddy_order() {
    let mut alloc = BuddyAllocator::new(BASE, 8, 3);

    let addr = alloc.alloc_pages(3);

    assert_eq!(addr, Some(BASE));
    assert_eq!(alloc.addr_order_map.get(&BASE), Some(&2));
    assert_eq!(alloc.free_pages_count(), 4);
    assert_eq!(alloc.alloc_pages(0), None);

    alloc.free(addr.unwrap());

    assert_eq!(alloc.free_pages_count(), 8);
}

#[test]
fn buddy_alloc_order_aligned_exact_hit() {
    let mut alloc = BuddyAllocator::new(BASE, 4, 2);

    let addr = alloc.alloc_order_aligned(2, 4);

    assert_eq!(addr, Some(BASE));
    assert_eq!(alloc.free_pages_count(), 0);
    assert_eq!(alloc.addr_order_map.get(&BASE), Some(&2));
    assert_eq!(alloc.statistics.alloc_count, 1);
    assert_eq!(alloc.statistics.exact_hit_count, 1);
    assert_eq!(alloc.statistics.split_count, 0);
}

#[test]
fn buddy_alloc_order_aligned_skips_unaligned_block() {
    let mut alloc = BuddyAllocator::new(BASE, 8, 3);
    assert_eq!(alloc.alloc_page(), Some(BASE));

    let addr = alloc.alloc_order_aligned(1, 4);

    assert_eq!(addr, Some(BASE + pages(4)));
    assert_eq!(((addr.unwrap() - BASE) / PAGE_SZ) % 4, 0);
    assert_eq!(alloc.free_pages_count(), 5);
    assert_eq!(alloc.addr_order_map.get(&(BASE + pages(4))), Some(&1));
    assert_eq!(alloc.free_lists[1], vec![BASE + pages(2), BASE + pages(6)]);
    assert_eq!(alloc.statistics.alloc_count, 2);
    assert_eq!(alloc.statistics.split_count, 4);
}

#[test]
fn buddy_alloc_order_aligned_failure_preserves_free_pages() {
    let mut alloc = BuddyAllocator::new(BASE, 8, 3);
    assert_eq!(alloc.alloc_order(2), Some(BASE));
    let free_before = alloc.free_pages_count();

    let addr = alloc.alloc_order_aligned(1, 8);

    assert_eq!(addr, None);
    assert_eq!(alloc.free_pages_count(), free_before);
    assert_eq!(alloc.statistics.failed_alloc_count, 1);
}

#[test]
fn buddy_free_without_merge_when_buddy_is_allocated() {
    let mut alloc = BuddyAllocator::new(BASE, 4, 2);
    let first = alloc.alloc_order(0).unwrap();
    let second = alloc.alloc_order(0).unwrap();

    alloc.free(first);
    alloc.free(first);
    alloc.free(BASE + 123);

    assert_eq!(first, BASE);
    assert_eq!(second, BASE + pages(1));
    assert_eq!(alloc.free_lists[0], vec![BASE]);
    assert_eq!(alloc.free_lists[1], vec![BASE + pages(2)]);
    assert_eq!(alloc.free_pages_count(), 3);
    assert_eq!(alloc.allocated.load(Ordering::Relaxed), 1);
    assert!(!alloc.addr_order_map.contains_key(&first));
    assert_eq!(alloc.addr_order_map.get(&second), Some(&0));
    assert_eq!(alloc.statistics.free_count, 1);
    assert_eq!(alloc.statistics.merge_count, 0);
}

#[test]
fn buddy_free_merges_buddy_chain() {
    let mut alloc = BuddyAllocator::new(BASE, 4, 2);
    let first = alloc.alloc_order(0).unwrap();
    let second = alloc.alloc_order(0).unwrap();

    alloc.free(first);
    alloc.free(second);

    assert_eq!(alloc.free_lists[0], Vec::<usize>::new());
    assert_eq!(alloc.free_lists[1], Vec::<usize>::new());
    assert_eq!(alloc.free_lists[2], vec![BASE]);
    assert_eq!(alloc.free_pages_count(), 4);
    assert_eq!(alloc.allocated.load(Ordering::Relaxed), 0);
    assert!(alloc.addr_order_map.is_empty());
    assert_eq!(alloc.statistics.free_count, 2);
    assert_eq!(alloc.statistics.merge_count, 2);
}

#[test]
fn buddy_free_rejects_invalid_recorded_blocks() {
    let mut alloc = BuddyAllocator::new(BASE, 4, 2);

    let before_free_pages = alloc.free_pages_count();

    let before_base = BASE - PAGE_SZ;
    alloc.addr_order_map.insert(before_base, 0);
    alloc.free(before_base);
    assert_eq!(alloc.addr_order_map.get(&before_base), Some(&0));

    let past_end = BASE + pages(4);
    alloc.addr_order_map.insert(past_end, 0);
    alloc.free(past_end);
    assert_eq!(alloc.addr_order_map.get(&past_end), Some(&0));

    let misaligned_to_order = BASE + pages(1);
    alloc.addr_order_map.insert(misaligned_to_order, 1);
    alloc.free(misaligned_to_order);
    assert_eq!(alloc.addr_order_map.get(&misaligned_to_order), Some(&1));

    assert_eq!(alloc.free_pages_count(), before_free_pages);
    assert_eq!(alloc.allocated.load(Ordering::Relaxed), 0);
    assert_eq!(alloc.statistics.free_count, 0);
    assert_eq!(alloc.statistics.merge_count, 0);
}

#[test]
fn buddy_snapshot_is_independent_copy() {
    let mut alloc = BuddyAllocator::new(BASE, 8, 3);
    let addr = alloc.alloc_order(1).unwrap();

    let snapshot = alloc.snapshot();
    alloc.free(addr);

    assert_eq!(snapshot.free_pages_count(), 6);
    assert_eq!(snapshot.largest_free_order(), 2);
    assert_eq!(snapshot.allocated.load(Ordering::Relaxed), 2);
    assert_eq!(snapshot.addr_order_map.get(&addr), Some(&1));
    assert_eq!(snapshot.statistics.alloc_count, 1);
    assert_eq!(snapshot.statistics.split_count, 2);

    assert_eq!(alloc.free_pages_count(), 8);
    assert_eq!(alloc.allocated.load(Ordering::Relaxed), 0);
    assert!(alloc.addr_order_map.is_empty());
}

#[test]
fn heuristic_allocator_standalone_interface_matches_buddy_baseline() {
    let mut alloc = HeuristicAllocator::new(BASE, 8, 3);

    assert_eq!(alloc.free_pages_count(), 8);
    assert_eq!(alloc.largest_free_order(), 3);
    assert_eq!(alloc.fragmentation_score(), 0);

    let first = alloc.alloc_page();
    let second = alloc.alloc_order_aligned(1, 4);

    assert_eq!(first, Some(BASE));
    assert_eq!(second, Some(BASE + pages(4)));
    assert_eq!(alloc.free_pages_count(), 5);
    assert!(!alloc.is_free_addr(BASE));
    assert!(alloc.is_free_addr(BASE + pages(2)));
    assert!(!alloc.is_free_addr(BASE + pages(4)));
    assert_eq!(alloc.addr_order_map.get(&BASE), Some(&0));
    assert_eq!(alloc.addr_order_map.get(&(BASE + pages(4))), Some(&1));
    assert_eq!(alloc.statistics.alloc_count, 2);
    assert_eq!(alloc.statistics.split_count, 4);
    assert_eq!(alloc.heuristic_statistics().fallback_alloc_count, 2);

    let snapshot = alloc.snapshot();
    alloc.free(first.unwrap());
    alloc.free(second.unwrap());

    assert_eq!(snapshot.free_pages_count(), 5);
    assert_eq!(snapshot.statistics.alloc_count, 2);
    assert_eq!(alloc.free_pages_count(), 8);
    assert_eq!(alloc.statistics.free_count, 2);
}

#[test]
fn heuristic_small_page_churn_keeps_cache_split_savings() {
    let mut buddy = BuddyAllocator::new(BASE, 1024, 10);
    for _ in 0..64 {
        let addr = buddy.alloc_page().unwrap();
        buddy.free(addr);
    }

    let mut alloc = HeuristicAllocator::new(BASE, 1024, 10);
    for _ in 0..64 {
        let addr = alloc.alloc_page().unwrap();
        alloc.free(addr);
    }

    let heuristic = alloc.heuristic_statistics();
    assert_eq!(alloc.free_pages_count(), 1024);
    assert_eq!(alloc.statistics.failed_alloc_count, 0);
    assert!(heuristic.cache_hit_count > 0);
    assert!(alloc.statistics.split_count < buddy.statistics.split_count);
}

#[test]
fn heuristic_large_pressure_bypasses_cache_to_restore_high_order_blocks() {
    let mut alloc = HeuristicAllocator::new(BASE, 64, 6);
    let mut live = Vec::new();

    for _ in 0..64 {
        live.push(alloc.alloc_page().unwrap());
    }
    for i in (0..64).step_by(2) {
        alloc.free(live[i]);
    }

    assert_eq!(alloc.alloc_order(5), None);
    assert!(alloc.merge_pressure);

    for i in (1..64).step_by(2) {
        alloc.free(live[i]);
    }

    let heuristic = alloc.heuristic_statistics();
    assert_eq!(alloc.free_pages_count(), 64);
    assert_eq!(alloc.largest_free_order(), 6);
    assert_eq!(alloc.fragmentation_score(), 0);
    assert_eq!(alloc.statistics.failed_alloc_count, 1);
    assert!(heuristic.cache_bypass_count > 0);
    assert_eq!(heuristic.pressure_enter_count, 1);
    assert_eq!(heuristic.pressure_exit_count, 1);
    assert!(!alloc.merge_pressure);
}

#[test]
fn benchmark_workload_small_page_churn_baseline() {
    let mut steps = Vec::new();
    for _ in 0..64 {
        steps.push(BenchmarkStep::AllocOrder(0));
        steps.push(BenchmarkStep::FreeNewest);
    }

    let report = run_buddy_benchmark("small_page_churn", 1024, 10, &steps);
    log_benchmark(&report);

    assert_eq!(report.attempted_ops, 128);
    assert_eq!(report.statistics.alloc_count, 64);
    assert_eq!(report.statistics.free_count, 64);
    assert_eq!(report.statistics.failed_alloc_count, 0);
    assert_eq!(report.statistics.split_count, 640);
    assert_eq!(report.statistics.merge_count, 640);
    assert_eq!(report.free_pages, 1024);
    assert_eq!(report.largest_free_order, 10);
    assert_eq!(report.fragmentation_score, 0);
    assert_eq!(report.live_blocks, 0);
}

#[test]
fn benchmark_workload_mixed_order_churn_baseline() {
    let orders = [0, 0, 1, 2, 0, 3, 1, 0, 2, 1, 0, 0, 3, 2, 1, 0];
    let mut steps = Vec::new();
    for i in 0..256 {
        if i % 17 == 0 {
            steps.push(BenchmarkStep::AllocAligned { order: 1, align: 4 });
        } else {
            steps.push(BenchmarkStep::AllocOrder(orders[i % orders.len()]));
        }
        if i % 3 == 2 {
            steps.push(BenchmarkStep::FreeOldest);
        }
    }
    steps.push(BenchmarkStep::FreeAll);

    let report = run_buddy_benchmark("mixed_order_churn", 2048, 11, &steps);
    log_benchmark(&report);

    assert_eq!(
        report.attempted_ops,
        report.statistics.alloc_count + report.statistics.failed_alloc_count + report.statistics.free_count
    );
    assert_eq!(report.statistics.alloc_count, 256);
    assert_eq!(report.statistics.free_count, 256);
    assert_eq!(report.statistics.failed_alloc_count, 0);
    assert!(report.statistics.split_count > 0);
    assert!(report.statistics.exact_hit_count > 0);
    assert_eq!(report.free_pages, 2048);
    assert_eq!(report.largest_free_order, 11);
    assert_eq!(report.fragmentation_score, 0);
    assert_eq!(report.live_blocks, 0);
}

#[test]
fn benchmark_workload_large_block_pressure_baseline() {
    let mut alloc = BuddyAllocator::new(BASE, 64, 6);
    let mut live = Vec::new();
    let mut attempted_ops = 0;
    let start = Instant::now();

    for _ in 0..64 {
        live.push(alloc.alloc_page().unwrap());
        attempted_ops += 1;
    }
    for i in (0..64).step_by(2) {
        alloc.free(live[i]);
        attempted_ops += 1;
    }

    let fragmented_free_pages = alloc.free_pages_count();
    let fragmented_largest_order = alloc.largest_free_order();
    let fragmented_score = alloc.fragmentation_score();
    let large = alloc.alloc_order(5);
    attempted_ops += 1;

    for i in (1..64).step_by(2) {
        alloc.free(live[i]);
        attempted_ops += 1;
    }

    let elapsed = start.elapsed();
    eprintln!(
        "large_block_pressure: ops={} elapsed={:?} alloc={} free={} split={} merge={} exact={} failed={} fragmented_free={} fragmented_largest_order={} fragmented_score={} final_free={} final_largest_order={} final_frag={}",
        attempted_ops,
        elapsed,
        alloc.statistics.alloc_count,
        alloc.statistics.free_count,
        alloc.statistics.split_count,
        alloc.statistics.merge_count,
        alloc.statistics.exact_hit_count,
        alloc.statistics.failed_alloc_count,
        fragmented_free_pages,
        fragmented_largest_order,
        fragmented_score,
        alloc.free_pages_count(),
        alloc.largest_free_order(),
        alloc.fragmentation_score(),
    );

    assert_eq!(large, None);
    assert_eq!(fragmented_free_pages, 32);
    assert_eq!(fragmented_largest_order, 0);
    assert_eq!(fragmented_score, 96);
    assert_eq!(alloc.statistics.alloc_count, 64);
    assert_eq!(alloc.statistics.free_count, 64);
    assert_eq!(alloc.statistics.failed_alloc_count, 1);
    assert_eq!(alloc.statistics.split_count, 63);
    assert_eq!(alloc.statistics.merge_count, 63);
    assert_eq!(alloc.free_pages_count(), 64);
    assert_eq!(alloc.largest_free_order(), 6);
    assert_eq!(alloc.fragmentation_score(), 0);
}

#[test]
fn frame_pool_get_inner_put_avail_and_free_count_use_buddy() {
    let pool = FramePool::new(2);

    assert_eq!(pool.free_count(), 2);
    assert!(pool.avail(0));
    assert!(pool.avail(1));
    assert!(!pool.avail(2));

    let first = pool.get_inner();
    let second = pool.get_inner();

    assert_eq!(first, Some(0));
    assert_eq!(second, Some(1));
    assert_eq!(pool.get_inner(), None);
    assert_eq!(pool.free_count(), 0);
    assert!(!pool.avail(0));
    assert!(!pool.avail(1));

    pool.put(0);
    assert_eq!(pool.free_count(), 1);
    assert!(pool.avail(0));
    assert!(!pool.avail(1));

    pool.put(99);
    assert_eq!(pool.free_count(), 1);

    pool.put(1);
    assert_eq!(pool.free_count(), 2);
    assert!(pool.avail(0));
    assert!(pool.avail(1));
}

#[test]
fn frame_pool_get_uses_public_locked_path() {
    let pool = FramePool::new(1);

    assert_eq!(pool.get(0xCAFE), Some(0));
    assert_eq!(pool.free_count(), 0);
    assert_eq!(pool.get(0xCAFE), None);

    pool.put(0);
    assert_eq!(pool.free_count(), 1);
}

#[test]
fn frame_pool_batch_alloc_returns_partial_non_contiguous_contract() {
    let pool = FramePool::new(4);

    let frames = pool.batch_alloc(6);

    assert_eq!(frames, vec![0, 1, 2, 3]);
    assert_eq!(pool.free_count(), 0);
    assert_eq!(pool.batch_alloc(1), Vec::<usize>::new());
}

#[test]
fn frame_pool_get_contig_allocates_whole_buddy_block() {
    let pool = FramePool::new(8);

    assert_eq!(pool.get_contig(0, 0), None);

    let start = pool.get_contig(3, 0);

    assert_eq!(start, Some(0));
    assert_eq!(pool.free_count(), 4);
    assert!(!pool.avail(0));
    assert!(!pool.avail(1));
    assert!(!pool.avail(2));
    assert!(!pool.avail(3));
    assert!(pool.avail(4));

    pool.put(start.unwrap());
    assert_eq!(pool.free_count(), 8);
}

#[test]
fn frame_pool_get_contig_uses_aligned_buddy_search() {
    let pool = FramePool::new(8);

    assert_eq!(pool.get_inner(), Some(0));

    let start = pool.get_contig(2, 2);

    assert_eq!(start, Some(4));
    assert_eq!(pool.free_count(), 5);
    assert!(!pool.avail(0));
    assert!(pool.avail(2));
    assert!(!pool.avail(4));
    assert!(!pool.avail(5));

    pool.put(start.unwrap());
    assert_eq!(pool.free_count(), 7);
    pool.put(0);
    assert_eq!(pool.free_count(), 8);
}

#[test]
fn frame_pool_zone_aware_alloc_and_free_updates_zone_count() {
    let pool = FramePool::new(8);
    let zone = ZoneInfo::new(0, 0, 4, 0, 4);

    let frame = pool.get_zone_aware(&zone);

    assert_eq!(frame, Some(0));
    assert_eq!(pool.free_count(), 7);
    assert_eq!(zone.free_count.load(Ordering::Relaxed), 3);

    pool.put_zone_aware(frame.unwrap(), &zone);

    assert_eq!(pool.free_count(), 8);
    assert_eq!(zone.free_count.load(Ordering::Relaxed), 4);
}

#[test]
fn frame_pool_zone_aware_rejects_pressure_and_wrong_zone() {
    let pool = FramePool::new(8);
    let blocked_zone = ZoneInfo::new(0, 0, 4, 4, 4);
    let high_zone = ZoneInfo::new(1, 4, 4, 0, 4);

    assert_eq!(pool.get_zone_aware(&blocked_zone), None);
    assert_eq!(blocked_zone.free_count.load(Ordering::Relaxed), 4);
    assert_eq!(pool.free_count(), 8);

    assert_eq!(pool.get_zone_aware(&high_zone), None);
    assert_eq!(high_zone.free_count.load(Ordering::Relaxed), 4);
    assert_eq!(pool.free_count(), 8);

    pool.put_zone_aware(0, &high_zone);
    assert_eq!(pool.free_count(), 8);
    assert_eq!(high_zone.free_count.load(Ordering::Relaxed), 4);
}

#[test]
fn frame_alloc_helpers_return_addresses_and_deallocate() {
    let pool = FramePool::new(4);

    let addr = frame_alloc(&pool);

    assert_eq!(addr, Some(MEM_OFF));
    assert_eq!(pool.free_count(), 3);

    frame_dealloc(&pool, MEM_OFF + 123);
    frame_dealloc(&pool, MEM_OFF - PAGE_SZ);
    assert_eq!(pool.free_count(), 3);

    frame_dealloc(&pool, addr.unwrap());
    assert_eq!(pool.free_count(), 4);

    let contig = frame_alloc_contig(&pool, 2, 0).unwrap();
    assert!(contig >= MEM_OFF);
    assert!(contig + pages(2) <= MEM_OFF + pages(4));
    assert_eq!((contig - MEM_OFF) % PAGE_SZ, 0);
    assert_eq!(pool.free_count(), 2);
    let start_frame = (contig - MEM_OFF) / PAGE_SZ;
    assert!(!pool.avail(start_frame));
    assert!(!pool.avail(start_frame + 1));

    frame_dealloc(&pool, contig);
    assert_eq!(pool.free_count(), 4);
}

#[test]
fn heap_grow_uses_frame_pool_allocator() {
    let pool = FramePool::new(4);

    let regions = heap_grow(&pool, 3);

    assert_eq!(regions, vec![(PHYS_OFF, 3 * PAGE_SZ)]);
    assert_eq!(pool.free_count(), 1);
}

#[test]
fn shared_page_fault_uses_frame_pool_allocator_once() {
    let pool = FramePool::new(2);
    let src = PgFrame::with_rc(2);
    let page = SharedPage::new(0);

    let resolved = page.fault(&pool, &src);

    assert_eq!(resolved, Ok(0));
    assert_eq!(pool.free_count(), 1);
    assert_eq!(src.count(), 1);
    assert!(page.is_cow_resolved());

    let second = page.fault(&pool, &src);
    assert_eq!(second, Ok(0));
    assert_eq!(pool.free_count(), 1);
}
