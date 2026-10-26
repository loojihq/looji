# Architecture

## Overview

The shop is one server and one storefront. The server owns accounts, carts, payments and invoices. The storefront
talks to it over a small JSON API.

## Accounts

Accounts sign in with a password. A successful sign in creates a session that expires after thirty minutes of
inactivity. Signing out ends the session at once.

## Payments

Card payments go through the payment provider. The server never stores card numbers. A refund returns money to the
same card and is recorded against the invoice.

## Reporting

A nightly script exports the sales figures to a spreadsheet for the finance team.
