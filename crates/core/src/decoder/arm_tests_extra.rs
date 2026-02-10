
    #[test]
    fn test_decode_bfi() {
        // BFI R0, R1, #4, #12
        // Encoding: T1
        // 1111 0011 0110 0001 0000 0000 1100 0110
        // h1 = F361, h2 = 00C6 ?
        // LSB=4, MSB=4+12-1 = 15.
        // imm3=1 (4>>2), imm2=0 (4&3).
        // msb = 15 = 01111 bin.
        // h2 = 0ii0 dddd iiim mmmm
        // i=0, d=0 (Rd=R0)
        // iii = imm3 = 001
        // mmmm = msb = 01111
        // h2 = 0000 0000 0010 1111 -> 0x002F ??

        // Let's manually construct:
        // Rd=0, Rn=1.
        // lsb=4 -> imm3=1, imm2=0.
        // width=12 -> msb=15 (0xF).
        // h1: 1111 0011 0110 0001 -> 0xF361
        // h2: 0ii0 dddd iiim mmmm -> 0000 0000 0010 1111 -> 0x002F

        assert_eq!(
            decode_thumb_32(0xF361, 0x002F),
            Instruction::Bfi { rd: 0, rn: 1, lsb: 4, width: 12 }
        );
    }

    #[test]
    fn test_decode_bfc() {
        // BFC R2, #8, #16
        // Rd=2, Rn=15 (0xF)
        // lsb=8 -> imm3=2, imm2=0.
        // width=16 -> msb=23 (0x17).
        // h1: 1111 0011 0110 1111 -> 0xF36F
        // h2: 0ii0 dddd iiim mmmm -> 0000 0010 0101 0111
        // d=2->0010. iii=010. mmmmm=10111 (23).
        // h2 -> 0x0257

        assert_eq!(
            decode_thumb_32(0xF36F, 0x0257),
            Instruction::Bfc { rd: 2, lsb: 8, width: 16 }
        );
    }

    #[test]
    fn test_decode_ubfx() {
        // UBFX R3, R4, #2, #5
        // Rd=3, Rn=4.
        // lsb=2 -> imm3=0, imm2=2.
        // width=5 -> widthm1=4.
        // h1: 1111 0011 1100 0100 -> 0xF3C4
        // h2: 0ii0 dddd iiww wwww
        // d=3->0011. iii=000. wwww=00100 -> 00100? No, widthm1 is 5 bits.
        // imm3=0 (2>>2). imm2=2 (2&3).
        // h2: 0000 0011 0100 0100 -> 0x0344
        // iii = imm3=0? wait.
        // lsb = (imm3<<2)|imm2. 2 = (0<<2)|2. Correct.
        // imm2 is at bit 6,7. iii at bit 12,13,14?
        // h2: 0ii0 dddd iiww wwww
        // imm3 is bits 14:12. imm2 bits 7:6.
        // 0000 0011 0(00)0 0(10)0 0100 -> 0x0344 is confusing.
        // Bit 14:12 = 000 -> imm3=0.
        // Bit 7:6 = 01 -> imm2=1? Wait. 0x44 -> 0100 0100.
        // bit 7:6 is 01 -> imm2=1. 1!=2.
        // Need imm2=2 -> 10 binary.
        // So bits 7:6 should be 10.
        // h2 -> ... 10 00100 -> 0x84?
        // Let's re-encode:
        // top: 0ii0 dddd
        // bot: iiww wwww
        // imm3=0. d=3. -> 0000 0011 -> 0x03..
        // imm2=2 -> 10. widthm1=4 -> 00100.
        // combined lower byte: 10 00100 -> 1000 0100 -> 0x84.
        // h2 = 0x0384.

        assert_eq!(
            decode_thumb_32(0xF3C4, 0x0384),
            Instruction::Ubfx { rd: 3, rn: 4, lsb: 2, width: 5 }
        );
    }

    #[test]
    fn test_decode_misc_rev() {
        // REV R1, R2
        // T2 encoding: 1111 1010 1001 mmmm 1111 dddd 1000 mmmm
        // h1: 1111 1010 1001 0010 -> FA92 (Rn in h1 is Rm=2)
        // h2: 1111 0001 1000 0010 -> F182 (Rd=1)

        assert_eq!(
            decode_thumb_32(0xFA92, 0xF182),
            Instruction::Rev { rd: 1, rm: 2 }
        );
    }
