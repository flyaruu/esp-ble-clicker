## Presentation clicker

This is an example of using HID over GATT/BLE to serve as a conference
clicker.

I've included some stl files, which might be useful if you also want to use a board with the same form factor.

This is the one I used: https://www.amazon.nl/dp/B0FKH2M99L?ref=ppx_yo2ov_dt_b_fed_asin_title&th=1

Mostly because it has an integrated charging circuit, which makes wiring the battery a whole lot easier. I was a bit underwhelmed with the range, here at home it seems alright, but on stage I only had about 3m range.

Some boards have external antenna capabilities, that might make a difference here.

If you have a different board and you want to port to it, it might be better to start a new project using esp-generate with the correct chip, and copy what's needed.
