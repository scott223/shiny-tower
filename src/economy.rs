#[derive(Debug)]
/// The tower's current balance.
pub struct Economy {
    /// Available funds.
    pub cash: i64,
}

impl Default for Economy {
    fn default() -> Self {
        Self { cash: 2_000_000 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cash_is_two_million() {
        assert_eq!(Economy::default().cash, 2_000_000);
    }
}
