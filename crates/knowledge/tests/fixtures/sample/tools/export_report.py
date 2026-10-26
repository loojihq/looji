import csv


def export_sales_report(rows, path):
    """Writes the month's sales figures to a spreadsheet file for the finance team."""
    with open(path, "w", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(["product", "units", "revenue"])
        for row in rows:
            writer.writerow([row["product"], row["units"], row["revenue"]])


def monthly_totals(rows):
    return sum(row["revenue"] for row in rows)
