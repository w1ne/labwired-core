/* EOS S3 M4 memory: one 512 KiB SRAM at 0x20000000, mirrored at 0x0
 * (Renode platforms/cpus/eos-s3.repl m4mem). Code links in the low view of the
 * lower half, data in the upper half, so the two never share bytes. The top
 * 256 bytes hold the smoke summary word (0x2007FF00). */
MEMORY
{
  FLASH : ORIGIN = 0x00000000, LENGTH = 256K
  RAM   : ORIGIN = 0x20040000, LENGTH = 256K - 256
}
