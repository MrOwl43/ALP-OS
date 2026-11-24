// SPDX-License-Identifier: AGPL-3.0-or-later
//! Long lists in pages (ADR 0028 #6): an archive can hold hundreds of
//! thousands of items, so a list keeps only a few pages in memory and asks
//! alpd for the page around what is on screen.

use std::collections::BTreeMap;

pub struct Pager<T> {
    page_size: usize,
    max_pages: usize,
    pages: BTreeMap<usize, Vec<T>>,
    last: usize,
}

impl<T> Pager<T> {
    /// Both values are at least 1.
    pub fn new(page_size: usize, max_pages: usize) -> Self {
        Pager {
            page_size: page_size.max(1),
            max_pages: max_pages.max(1),
            pages: BTreeMap::new(),
            last: 0,
        }
    }

    fn page_of(&self, index: usize) -> usize {
        index / self.page_size
    }

    /// The page to fetch for `index`, if it is not loaded. Remembers `index`
    /// as the place the user is looking at.
    pub fn want(&mut self, index: usize) -> Option<usize> {
        self.last = index;
        let p = self.page_of(index);
        (!self.pages.contains_key(&p)).then_some(p)
    }

    /// Stores a fetched page; when full, drops the page farthest from the
    /// place last asked for (never the page just stored).
    pub fn put(&mut self, page: usize, items: Vec<T>) {
        self.pages.insert(page, items);
        let here = self.page_of(self.last);
        while self.pages.len() > self.max_pages {
            let far = self
                .pages
                .keys()
                .copied()
                .filter(|&p| p != page)
                .max_by_key(|&p| p.abs_diff(here));
            match far {
                Some(p) => {
                    self.pages.remove(&p);
                }
                None => break,
            }
        }
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.pages
            .get(&self.page_of(index))?
            .get(index % self.page_size)
    }

    /// Loaded pages, for tests and memory checks.
    pub fn loaded(&self) -> usize {
        self.pages.len()
    }

    /// Forget everything (the list changed on the server).
    pub fn clear(&mut self) {
        self.pages.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(p: usize, size: usize) -> Vec<usize> {
        (p * size..(p + 1) * size).collect()
    }

    #[test]
    fn asks_once_then_serves_from_memory() {
        let mut g = Pager::new(100, 3);
        assert_eq!(g.want(250), Some(2));
        g.put(2, page(2, 100));
        assert_eq!(g.want(299), None);
        assert_eq!(g.get(250), Some(&250));
        assert_eq!(g.get(300), None);
    }

    #[test]
    fn drops_the_farthest_page() {
        let mut g = Pager::new(10, 3);
        for p in [0, 1, 2] {
            g.want(p * 10);
            g.put(p, page(p, 10));
        }
        // The user jumped to page 5: page 0 is farthest and goes.
        assert_eq!(g.want(55), Some(5));
        g.put(5, page(5, 10));
        assert_eq!(g.loaded(), 3);
        assert_eq!(g.get(5), None);
        assert_eq!(g.get(15), Some(&15));
        assert_eq!(g.get(55), Some(&55));
    }

    #[test]
    fn short_last_page_and_huge_lists() {
        let mut g: Pager<u64> = Pager::new(500, 4);
        let last = 299_999usize;
        assert_eq!(g.want(last), Some(599));
        g.put(599, vec![7; 500]);
        assert_eq!(g.get(last), Some(&7));
        g.put(598, vec![1; 3]);
        assert_eq!(g.get(598 * 500 + 2), Some(&1));
        assert_eq!(g.get(598 * 500 + 3), None);
    }

    #[test]
    fn zero_sizes_are_safe() {
        let mut g = Pager::new(0, 0);
        assert_eq!(g.want(3), Some(3));
        g.put(3, vec!['a']);
        g.put(4, vec!['b']);
        assert_eq!(g.loaded(), 1);
        assert_eq!(g.get(4), Some(&'b'));
    }
}
