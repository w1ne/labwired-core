# Original linker-map collector — revised qualification pending

The first [run37759168804](https://github.com/CrispStrobe/labwired-core/actions/runs/37759168804)
failed admission at source `5f04b8b226e7185968a3be53f346cb65e85f6509`:
the original command already has `-Wl,-Map,ITSYBITSY_M4.map`. No diagnostic
linker ran. Preserve that failure and the actual recipe from
[artifact11542717263](https://api.github.com/repos/CrispStrobe/labwired-core/actions/artifacts/11542717263/zip),
ZIP 410,994 bytes, SHA256
`c193480a790d12a91451cdfcaf1f3f02963c71a70980323f7a36df481cca1dda`.
The recipe member SHA256 is
`8163427829709d67f4667c35c5ed7e8e8f31cdd701b96cca357246781c90d264`.
That artifact contains no map; do not invent its bytes from the recipe.

The revised collector accepts exactly the observed original map/output contract
and captures the original clean build's existing map, linker script and recipe.
No relink, firmware execution or disassembly is needed. It hashes the original
ELF without uploading it, checks it remains unchanged, rejects missing/linked
outputs, ambiguous recipes and changed map/script/output paths, and retains raw
evidence without normalization. The first revised hosted result is pending.

Require all enabled exact-head checks and independent raw-map audit before
landing. Map/image hash observations do not independently reproduce the linker
or establish complete source provenance: headers, inline code, transitive linker
script inputs, component obligations and actual retained/discarded sections
remain separate R0/R1 review work. No licence classification or app pin changes.
