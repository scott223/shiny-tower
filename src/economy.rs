#[derive(Debug, Default)]
/// The tower's current balance and recurring financial totals.
pub struct Economy {
    /// Available funds.
    pub cash: i64,
    /// Income earned per day.
    pub daily_income: i64,
    /// Expenses paid per day.
    pub daily_expenses: i64,
}
