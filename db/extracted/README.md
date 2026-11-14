# Extracted test conditions (2026-11-14)

`conditions.json` holds 454 test conditions read from the NTRS papers listed in the `papers` table,
for import into the `conditions` table. It was produced outside the backend pipeline, by the two
scripts in this folder, using `claude-opus-5-5`.

How each row was checked:

1. `extract.py` downloads each paper's PDF from NTRS, reads it page by page, and asks the model for
   the microgravity or spaceflight tests the paper reports. A row is kept only if its `source_quote`
   is found word for word in the PDF text (compared with spaces and punctuation removed, because PDF
   text breaks spacing) and every oxygen figure in the row appears on the cited page.
2. `verify.py` sends each row that claims an outcome back with the cited page and its neighbours.
   The outcome is kept only if the model confirms it and supplies an `outcome_source_quote` that is
   also found word for word. Otherwise the outcome is `unknown`.

Rows the model marked as inferred, or below 0.5 confidence, were removed. Nothing here has been
reviewed by a person: every row is imported as `verification_status = 'pending'`.

Known limits: outcomes are the model's reading of the quoted sentence; fuel names are as the papers
write them and are not normalised; two poster PDFs had no readable text and produced no rows.
