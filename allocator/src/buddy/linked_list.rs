//! Sorted linked list implementation for buddy allocator free lists
//!
//! Provides O(n) insertion with sorted order (by address), enabling efficient
//! contiguity checking for composite allocations.

use core::cmp::PartialOrd;
use log::warn;

#[cfg(test)]
extern crate alloc;

/// Simple static linked list node
#[derive(Debug, Clone, Copy)]
pub(crate) struct ListNode<T> {
    pub(crate) data: T,
    pub(crate) next: Option<usize>,
}

/// Static linked list implementation that doesn't require dynamic allocation
#[derive(Debug)]
pub struct StaticLinkedList<T, const N: usize> {
    pub(crate) nodes: [Option<ListNode<T>>; N],
    pub(crate) head: Option<usize>,
    pub(crate) tail: Option<usize>,
    pub(crate) free_head: Option<usize>,
    pub(crate) len: usize,
}

impl<T, const N: usize> StaticLinkedList<T, N> {
    /// Create a new empty static linked list
    pub const fn new() -> Self {
        Self {
            nodes: [const { None }; N],
            head: None,
            tail: None,
            free_head: Some(0),
            len: 0,
        }
    }

    /// Initialize the free list
    pub fn init(&mut self) {
        // Initialize all nodes as free
        for i in 0..N {
            debug_assert!(i < N, "Index out of bounds in StaticLinkedList::init");

            self.nodes[i] = Some(ListNode {
                data: unsafe { core::mem::zeroed() },
                next: if i < N - 1 { Some(i + 1) } else { None },
            });
        }
        self.free_head = Some(0);
        self.head = None;
        self.tail = None;
        self.len = 0;
    }

    /// Push an element to the back of the list (not sorted)
    pub fn push_back(&mut self, data: T) -> bool {
        if self.free_head.is_none() {
            return false;
        }

        let new_node_idx = self.free_head.unwrap();
        if new_node_idx >= N {
            return false;
        }

        let next_free = self.nodes[new_node_idx].as_ref()
            .map(|n| n.next).unwrap_or(None);

        self.nodes[new_node_idx] = Some(ListNode {
            data,
            next: None,
        });

        self.free_head = next_free;

        if self.tail.is_none() {
            self.head = Some(new_node_idx);
            self.tail = Some(new_node_idx);
        } else {
            let tail_idx = self.tail.unwrap();
            if tail_idx >= N {
                return false;
            }

            if let Some(tail_node) = self.nodes[tail_idx].as_mut() {
                tail_node.next = Some(new_node_idx);
            } else {
                panic!("Tail node {} is corrupted", tail_idx);
            }
            self.tail = Some(new_node_idx);
        }

        self.len += 1;
        true
    }

    /// Insert element in sorted order (ascending by address)
    /// This is used for buddy free lists to enable efficient contiguity checking
    pub fn insert_sorted(&mut self, data: T) -> bool
    where
        T: PartialOrd,
    {
        if self.free_head.is_none() {
            return false;
        }

        let new_node_idx = self.free_head.unwrap();
        if new_node_idx >= N {
            return false;
        }

        let next_free = self.nodes[new_node_idx].as_ref()
            .map(|n| n.next).unwrap_or(None);

        // Find insertion position
        let mut prev_idx = None;
        let mut current_idx = self.head;

        while let Some(idx) = current_idx {
            if let Some(node) = &self.nodes[idx] {
                if node.data > data {
                    break; // Found position
                }
                prev_idx = current_idx;
                current_idx = node.next;
            } else {
                break;
            }
        }

        // Insert the new node
        self.nodes[new_node_idx] = Some(ListNode {
            data,
            next: current_idx,
        });

        // Update links
        if let Some(prev) = prev_idx {
            if let Some(prev_node) = self.nodes[prev].as_mut() {
                prev_node.next = Some(new_node_idx);
            }
        } else {
            self.head = Some(new_node_idx);
        }

        // Update tail if needed
        if current_idx.is_none() {
            self.tail = Some(new_node_idx);
        }

        self.free_head = next_free;
        self.len += 1;
        true
    }

    /// Pop an element from the front of the list
    pub fn pop_front(&mut self) -> Option<T> {
        if self.head.is_none() {
            return None;
        }

        let head_idx = self.head.unwrap();
        if head_idx >= N {
            panic!("Head node {} is corrupted", head_idx);
        }

        let head_node = self.nodes[head_idx].take()?;

        self.head = head_node.next;
        if self.head.is_none() {
            self.tail = None;
        }

        // Return node to free list
        let node = ListNode {
            data: unsafe { core::mem::zeroed() },
            next: self.free_head,
        };
        self.nodes[head_idx] = Some(node);
        self.free_head = Some(head_idx);

        self.len -= 1;
        Some(head_node.data)
    }

    /// Check if the list is empty
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Get the length of the list
    pub fn len(&self) -> usize {
        self.len
    }

    /// Get iterator over elements
    pub fn iter(&self) -> Iter<'_, T, N> {
        Iter {
            list: self,
            current: self.head,
        }
    }

    /// Find a node by address (for buddy system)
    pub fn find_by_addr(&self, addr: usize) -> Option<usize> {
        let mut current_idx = self.head;
        let mut visited = 0;

        while let Some(idx) = current_idx {
            if visited > self.len {
                warn!("Potential cycle detected during search");
                return None;
            }

            if let Some(node) = &self.nodes[idx] {
                // For BuddyBlock, check address
                unsafe {
                    let node_data = &node.data as *const T as *const (usize, usize);
                    if (*node_data).1 == addr {
                        return Some(idx);
                    }
                }
                current_idx = node.next;
            } else {
                break;
            }
            visited += 1;
        }

        None
    }

    /// Remove a node at the given index
    pub fn remove(&mut self, node_idx: usize) {
        if node_idx >= N || self.nodes[node_idx].is_none() {
            return;
        }

        let mut prev_idx = None;
        let mut current_idx = self.head;
        let mut visited = 0;

        while let Some(idx) = current_idx {
            if visited > self.len {
                warn!("Potential cycle detected");
                return;
            }

            if idx == node_idx {
                break;
            }
            prev_idx = current_idx;
            if let Some(node) = &self.nodes[idx] {
                current_idx = node.next;
            } else {
                break;
            }
            visited += 1;
        }

        if current_idx != Some(node_idx) {
            return;
        }

        if let Some(node) = self.nodes[node_idx].take() {
            if let Some(prev_idx) = prev_idx {
                if let Some(prev_node) = &mut self.nodes[prev_idx] {
                    prev_node.next = node.next;
                }
            } else {
                self.head = node.next;
            }

            if self.tail == Some(node_idx) {
                self.tail = prev_idx;
            } else if self.head.is_none() {
                self.tail = None;
            }

            let dummy_node = ListNode {
                data: unsafe { core::mem::zeroed() },
                next: self.free_head,
            };
            self.nodes[node_idx] = Some(dummy_node);
            self.free_head = Some(node_idx);
            self.len -= 1;
        }
    }
}

/// Iterator for StaticLinkedList
pub struct Iter<'a, T, const N: usize> {
    list: &'a StaticLinkedList<T, N>,
    current: Option<usize>,
}

impl<'a, T, const N: usize> Iterator for Iter<'a, T, N> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        self.current.and_then(|idx| {
            if let Some(node) = &self.list.nodes[idx] {
                self.current = node.next;
                Some(&node.data)
            } else {
                self.current = None;
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linked_list_basic() {
        let mut list: StaticLinkedList<usize, 10> = StaticLinkedList::new();
        list.init();

        assert!(list.is_empty());
        assert_eq!(list.len(), 0);

        list.push_back(1);
        list.push_back(2);
        list.push_back(3);

        assert_eq!(list.len(), 3);
        assert_eq!(list.pop_front(), Some(1));
        assert_eq!(list.pop_front(), Some(2));
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn test_insert_sorted() {
        let mut list: StaticLinkedList<usize, 10> = StaticLinkedList::new();
        list.init();

        list.insert_sorted(5);
        list.insert_sorted(3);
        list.insert_sorted(7);
        list.insert_sorted(1);

        let items: alloc::vec::Vec<_> = list.iter().collect();
        assert_eq!(items, alloc::vec![&1, &3, &5, &7]);
    }
}
