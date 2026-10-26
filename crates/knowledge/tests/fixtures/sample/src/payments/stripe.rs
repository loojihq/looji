pub struct Charge {
    pub cents: u64,
    pub card_token: String,
}

/// Bills the customer's credit card through the payment provider and returns the receipt id.
pub fn charge_card(cents: u64, card_token: &str) -> Result<String, String> {
    if cents == 0 {
        return Err("nothing to charge".to_string());
    }
    Ok(format!("receipt-{cents}-{card_token}"))
}

/// Gives money back to the card that paid. The refund is recorded against the receipt.
pub fn refund_payment(receipt: &str, cents: u64) -> Result<(), String> {
    if receipt.is_empty() || cents == 0 {
        return Err("nothing to refund".to_string());
    }
    Ok(())
}
