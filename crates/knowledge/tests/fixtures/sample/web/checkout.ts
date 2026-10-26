export interface Address {
  street: string;
  city: string;
  postcode: string;
}

// The shipping address needs a street, a city, and a postcode of five digits.
export function validateAddress(address: Address): string[] {
  const problems: string[] = [];
  if (!address.street.trim()) problems.push("Enter the street");
  if (!address.city.trim()) problems.push("Enter the city");
  if (!/^\d{5}$/.test(address.postcode)) problems.push("The postcode has five digits");
  return problems;
}

export async function submitCheckout(address: Address, cardToken: string): Promise<boolean> {
  if (validateAddress(address).length > 0) return false;
  const response = await fetch("/api/checkout", { method: "POST", body: JSON.stringify({ address, cardToken }) });
  return response.ok;
}
