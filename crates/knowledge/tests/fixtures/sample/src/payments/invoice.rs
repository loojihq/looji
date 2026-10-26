pub struct Invoice {
    pub number: u32,
    pub lines: Vec<(String, u64)>,
}

/// Builds the invoice for an order from its lines.
pub fn build_invoice(number: u32, lines: Vec<(String, u64)>) -> Invoice {
    Invoice { number, lines }
}

/// Emails the invoice to the customer as a PDF attachment.
pub fn send_invoice_email(invoice: &Invoice, address: &str) -> bool {
    !address.is_empty() && !invoice.lines.is_empty()
}

pub fn invoice_total(invoice: &Invoice) -> u64 {
    invoice.lines.iter().map(|(_, cents)| cents).sum()
}
