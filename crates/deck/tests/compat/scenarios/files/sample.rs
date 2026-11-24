use std::collections::HashMap;

/// A tiny inventory of named items.
#[derive(Debug, Default)]
struct Inventory {
    items: HashMap<String, u32>,
}

impl Inventory {
    fn add(&mut self, name: &str, count: u32) {
        *self.items.entry(name.to_string()).or_insert(0) += count;
    }

    fn total(&self) -> u32 {
        self.items.values().sum()
    }
}

fn main() {
    let mut inventory = Inventory::default();
    inventory.add("apple", 3);
    inventory.add("pear", 5);
    // Print the total, then every item.
    println!("total: {}", inventory.total());
    for (name, count) in &inventory.items {
        println!("{name}: {count}");
    }
}
