class Invoice:
    def __init__(self, customer, items):
        self.customer = customer
        self.items = items


def total_due(invoice):
    return sum(item["price"] for item in invoice.items)


def create_invoice(customer, items):
    return Invoice(customer, items)
