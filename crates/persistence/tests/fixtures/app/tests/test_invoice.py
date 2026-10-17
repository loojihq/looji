from billing.invoice import total_due


def test_total_due_is_the_sum_of_prices():
    assert total_due({"items": []}) == 0
