# Word list licences

`dictionary.txt` — every word the game accepts — is the union of two lists:

- **ENABLE** (Enhanced North American Benchmark Lexicon), placed in the public
  domain by its authors.
- **SCOWL** (Spell Checker Oriented Word Lists) by Kevin Atkinson, at size 95,
  version 2020.12.07, from http://wordlist.aspell.net/. SCOWL may be used, copied
  and distributed provided its copyright notice and permission notice appear in
  all copies; they are reproduced in full in `SCOWL-Copyright.txt`.

Only lowercase words made of the letters a–z are included, and what SCOWL adds is
filtered against SCOWL's own lists: no acronyms or initialisms, no names of
people, places or brands, and no foreign words. Days, months and festivals are
kept. Nothing is removed from ENABLE.

`common.txt`, the everyday tier boards are built from, is SCOWL at a smaller size
intersected with the accepted list.
