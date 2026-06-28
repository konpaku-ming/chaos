// created by claude
use chaos_tests::*;
use std::sync::atomic::Ordering;

const BASE: usize = 0x1000_0000;

fn pages(n: usize) -> usize {
    n * PAGE_SZ
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
fn buddy_free_without_merge_when_buddy_is_allocated() {
    let mut alloc = BuddyAllocator::new(BASE, 4, 2);
    let first = alloc.alloc_order(0).unwrap();
    let second = alloc.alloc_order(0).unwrap();

    alloc.free_order(first);
    alloc.free_order(first);
    alloc.free_order(BASE + 123);

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

    alloc.free_order(first);
    alloc.free_order(second);

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
    alloc.free_order(before_base);
    assert_eq!(alloc.addr_order_map.get(&before_base), Some(&0));

    let past_end = BASE + pages(4);
    alloc.addr_order_map.insert(past_end, 0);
    alloc.free_order(past_end);
    assert_eq!(alloc.addr_order_map.get(&past_end), Some(&0));

    let misaligned_to_order = BASE + pages(1);
    alloc.addr_order_map.insert(misaligned_to_order, 1);
    alloc.free_order(misaligned_to_order);
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
    alloc.free_order(addr);

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
fn frame_pool_get_contig_alignment_failure_restores_state() {
    let pool = FramePool::new(8);

    assert_eq!(pool.get_inner(), Some(0));
    let free_before = pool.free_count();

    assert_eq!(pool.get_contig(2, 2), None);
    assert_eq!(pool.free_count(), free_before);
    assert!(!pool.avail(0));
    assert!(pool.avail(2));

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

    let contig = frame_alloc_contig(&pool, 2, 0);
    assert_eq!(contig, Some(MEM_OFF));
    assert_eq!(pool.free_count(), 2);

    frame_dealloc(&pool, contig.unwrap());
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
