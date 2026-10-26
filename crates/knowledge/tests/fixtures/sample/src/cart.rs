pub struct Cart {
    pub items: Vec<(String, u64, u32)>,
}

/// Puts a product in the shopping cart, or adds to its quantity when it is already there.
pub fn add_to_cart(cart: &mut Cart, product: &str, cents: u64) {
    if let Some(line) = cart.items.iter_mut().find(|line| line.0 == product) {
        line.2 += 1;
    } else {
        cart.items.push((product.to_string(), cents, 1));
    }
}

pub fn remove_from_cart(cart: &mut Cart, product: &str) {
    cart.items.retain(|line| line.0 != product);
}

/// The price of everything in the cart, in cents.
pub fn cart_total(cart: &Cart) -> u64 {
    cart.items.iter().map(|line| line.1 * u64::from(line.2)).sum()
}
