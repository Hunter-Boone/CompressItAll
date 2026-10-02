//! Budget allocation for a per-message limit (DESIGN.md 3.9.1): water-filling
//! between each item's lossless size `l` and its quality-floor size `f`, plus
//! first-fit-decreasing split suggestions when even the floors do not fit.

#[derive(Debug, Clone, PartialEq)]
pub struct ItemSizes {
    pub id: String,
    /// Best lossless/optimised size (or the original when nothing helps).
    pub lossless: u64,
    /// Size at the quality floor at full size. Equal to `lossless` for fixed items.
    pub floor: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Allocation {
    /// Every item takes its lossless size.
    AllLossless(Vec<(String, u64)>),
    /// Water-filled budgets, one per item, in input order.
    Budgets(Vec<(String, u64)>),
    /// Even the floors exceed the budget.
    TooBig {
        split: Vec<Vec<String>>,
        remove: Vec<String>,
    },
}

/// Allocate `budget` across items. Returns per-item budgets that never go below
/// an item's floor and never sum above `budget`.
pub fn allocate(items: &[ItemSizes], budget: u64) -> Allocation {
    let sum_l: u64 = items.iter().map(|i| i.lossless).sum();
    if sum_l <= budget {
        return Allocation::AllLossless(items.iter().map(|i| (i.id.clone(), i.lossless)).collect());
    }
    let sum_f: u64 = items.iter().map(|i| i.floor.min(i.lossless)).sum();
    if sum_f > budget {
        return Allocation::TooBig {
            split: first_fit_decreasing(items, budget),
            remove: removal_list(items, budget),
        };
    }
    // Water-fill: b_i = f_i + spare * (l_i - f_i) / sum(l - f), clamp to l_i, redistribute.
    let n = items.len();
    let mut b: Vec<u64> = items.iter().map(|i| i.floor.min(i.lossless)).collect();
    let mut capped = vec![false; n];
    let mut spare = budget - sum_f;
    for _ in 0..n + 1 {
        if spare == 0 {
            break;
        }
        let room: u128 = items
            .iter()
            .enumerate()
            .filter(|(k, _)| !capped[*k])
            .map(|(k, i)| (i.lossless - b[k]) as u128)
            .sum();
        if room == 0 {
            break;
        }
        let mut given = 0u64;
        let mut newly_capped = false;
        for (k, i) in items.iter().enumerate() {
            if capped[k] {
                continue;
            }
            let r = (i.lossless - b[k]) as u128;
            let share = ((spare as u128) * r / room) as u64;
            let add = share.min(i.lossless - b[k]);
            b[k] += add;
            given += add;
            if b[k] >= i.lossless {
                capped[k] = true;
                newly_capped = true;
            }
        }
        spare -= given;
        if !newly_capped {
            break;
        }
    }
    // Rounding dust: hand leftover to the first uncapped items, one byte at a time.
    for (k, i) in items.iter().enumerate() {
        if spare == 0 {
            break;
        }
        let add = (i.lossless - b[k]).min(spare);
        b[k] += add;
        spare -= add;
    }
    Allocation::Budgets(
        items
            .iter()
            .zip(b)
            .map(|(i, v)| (i.id.clone(), v))
            .collect(),
    )
}

/// Bin-pack items by their floor size into bins of `bin` bytes, largest first.
/// An item whose floor exceeds one bin gets a bin of its own (the UI will show it as refused).
pub fn first_fit_decreasing(items: &[ItemSizes], bin: u64) -> Vec<Vec<String>> {
    let mut sorted: Vec<&ItemSizes> = items.iter().collect();
    sorted.sort_by(|a, b| b.floor.cmp(&a.floor).then_with(|| a.id.cmp(&b.id)));
    let mut bins: Vec<(u64, Vec<String>)> = Vec::new();
    for it in sorted {
        let size = it.floor.min(it.lossless);
        if let Some(slot) = bins.iter_mut().find(|(used, _)| *used + size <= bin) {
            slot.0 += size;
            slot.1.push(it.id.clone());
        } else {
            bins.push((size, vec![it.id.clone()]));
        }
    }
    bins.into_iter().map(|(_, ids)| ids).collect()
}

/// The largest items whose removal makes the rest fit at the floor.
pub fn removal_list(items: &[ItemSizes], budget: u64) -> Vec<String> {
    let mut sorted: Vec<&ItemSizes> = items.iter().collect();
    sorted.sort_by_key(|i| std::cmp::Reverse(i.floor));
    let mut total: u64 = items.iter().map(|i| i.floor.min(i.lossless)).sum();
    let mut out = Vec::new();
    for it in sorted {
        if total <= budget {
            break;
        }
        total -= it.floor.min(it.lossless);
        out.push(it.id.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn it(id: &str, l: u64, f: u64) -> ItemSizes {
        ItemSizes {
            id: id.into(),
            lossless: l,
            floor: f,
        }
    }
    #[test]
    fn all_lossless_when_it_fits() {
        let a = allocate(&[it("a", 100, 10), it("b", 200, 20)], 300);
        assert_eq!(
            a,
            Allocation::AllLossless(vec![("a".into(), 100), ("b".into(), 200)])
        );
    }
    #[test]
    fn water_fill_respects_floor_and_sum() {
        let items = [it("a", 1000, 100), it("b", 400, 100), it("c", 100, 100)];
        match allocate(&items, 900) {
            Allocation::Budgets(b) => {
                let sum: u64 = b.iter().map(|x| x.1).sum();
                assert!(sum <= 900, "sum {sum}");
                assert!(sum >= 890, "should use nearly all of it: {sum}");
                for (x, i) in b.iter().zip(items.iter()) {
                    assert!(x.1 >= i.floor && x.1 <= i.lossless, "{:?}", x);
                }
                // c is fixed at its floor, b and a share proportionally
                assert_eq!(b[2].1, 100);
                assert!(b[0].1 > b[1].1);
            }
            other => panic!("{other:?}"),
        }
    }
    #[test]
    fn redistribution_terminates_and_caps() {
        // spare is split in proportion to how much each item can still use; the sum is exact
        let items = [it("a", 10_000, 100), it("b", 150, 100)];
        match allocate(&items, 5000) {
            Allocation::Budgets(b) => {
                assert_eq!(b[0].1 + b[1].1, 5000);
                assert_eq!(b[1].1, 124);
                assert_eq!(b[0].1, 4876);
            }
            other => panic!("{other:?}"),
        }
    }
    #[test]
    fn too_big_gives_split_and_removal() {
        let items = [it("a", 900, 600), it("b", 800, 500), it("c", 700, 400)];
        match allocate(&items, 1000) {
            Allocation::TooBig { split, remove } => {
                assert_eq!(
                    split,
                    vec![
                        vec!["a".to_string(), "c".to_string()],
                        vec!["b".to_string()]
                    ]
                );
                assert_eq!(remove, vec!["a".to_string()]);
            }
            other => panic!("{other:?}"),
        }
    }
    #[test]
    fn ffd_never_overfills() {
        let items: Vec<ItemSizes> = (0..40)
            .map(|k| it(&format!("i{k}"), 100 + k * 7, 50 + k * 7))
            .collect();
        for bin in [120u64, 300, 1000] {
            for group in first_fit_decreasing(&items, bin) {
                let total: u64 = group
                    .iter()
                    .map(|id| items.iter().find(|i| &i.id == id).unwrap().floor)
                    .sum();
                assert!(total <= bin || group.len() == 1);
            }
        }
    }
}
