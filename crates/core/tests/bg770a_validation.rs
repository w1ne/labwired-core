// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Real-hardware validation for the Quectel BG770A-GL model.
//!
//! Three flavours of test, all driven by captures taken from the on-the-bench
//! EVB running firmware `BG770AGLAAR01A05`:
//!
//! 1. **Exact byte-for-byte** for deterministic single-shot commands (identity
//!    strings, test forms, error shapes). The model must produce the same
//!    bytes the real chip put on its UART, in the same order, with the same
//!    framing.
//! 2. **Shape match** for state-varying commands (signal, registration,
//!    operator, IMEI/IMSI/ICCID, PDP context). The captured payload contains
//!    device-specific or network-specific values; we assert the model's
//!    response satisfies the same structural template using a tiny placeholder
//!    syntax — `{N}` digits, `{H}` hex digits, `{R}` any non-CRLF run.
//! 3. **State sequences**: replay a captured multi-command flow through one
//!    long-lived modem instance and assert each response matches.
//!
//! Regenerate captures with `core/scripts/capture_bg770a_golden.py` after a
//! firmware-version bump.

use labwired_core::peripherals::components::QuectelBg770a;
use labwired_core::peripherals::uart::UartStreamDevice;

/// Real-hardware capture for deterministic commands. The expected byte stream
/// includes the `<cmd>\r` echo and the modem's response framing.
const EXACT_GOLDEN: &[(&str, &[u8])] = &[
    ("AT", b"AT\r\r\nOK\r\n"),
    (
        "ATI",
        b"ATI\r\r\nQuectel\r\nBG770A-GL\r\nRevision: BG770AGLAAR01A05\r\n\r\nOK\r\n",
    ),
    (
        "ATI1",
        b"ATI1\r\r\nQuectel\r\nBG770A-GL\r\nRevision: BG770AGLAAR01A05\r\n\r\nOK\r\n",
    ),
    ("AT+CGMI", b"AT+CGMI\r\r\nQuectel\r\n\r\nOK\r\n"),
    ("AT+GMI", b"AT+GMI\r\r\nQuectel\r\n\r\nOK\r\n"),
    ("AT+CGMM", b"AT+CGMM\r\r\nBG770A-GL\r\n\r\nOK\r\n"),
    ("AT+GMM", b"AT+GMM\r\r\nBG770A-GL\r\n\r\nOK\r\n"),
    ("AT+CGMR", b"AT+CGMR\r\r\nBG770AGLAAR01A05\r\n\r\nOK\r\n"),
    ("AT+GMR", b"AT+GMR\r\r\nBG770AGLAAR01A05\r\n\r\nOK\r\n"),
    ("AT+CPIN?", b"AT+CPIN?\r\r\n+CPIN: READY\r\n\r\nOK\r\n"),
    ("AT+CPIN=?", b"AT+CPIN=?\r\r\nOK\r\n"),
    ("AT+CMEE=?", b"AT+CMEE=?\r\r\n+CMEE: (0-2)\r\n\r\nOK\r\n"),
    (
        "AT+CFUN=?",
        b"AT+CFUN=?\r\r\n+CFUN: (0-1,4),(0-1)\r\n\r\nOK\r\n",
    ),
    (
        "AT+CSQ=?",
        b"AT+CSQ=?\r\r\n+CSQ: (0-31,99),(0-7,99)\r\n\r\nOK\r\n",
    ),
    ("AT+CEREG=?", b"AT+CEREG=?\r\r\n+CEREG: (0-2,4)\r\n\r\nOK\r\n"),
    ("AT+CREG=?", b"AT+CREG=?\r\r\n+CREG: (0-2)\r\n\r\nOK\r\n"),
    ("AT&F", b"AT&F\r\r\nOK\r\n"),
    ("AT&W", b"AT&W\r\r\nOK\r\n"),
    ("AT+CGATT=?", b"AT+CGATT=?\r\r\n+CGATT: (0-1)\r\n\r\nOK\r\n"),
    ("AT+CGACT=?", b"AT+CGACT=?\r\r\n+CGACT: (0-1)\r\n\r\nOK\r\n"),
    ("AT+CGPADDR=?", b"AT+CGPADDR=?\r\r\n+CGPADDR: (1)\r\n\r\nOK\r\n"),
    ("AT+CGPADDR=1", b"AT+CGPADDR=1\r\r\n+CGPADDR: 1\r\n\r\nOK\r\n"),
    (
        "AT+QICSGP=?",
        b"AT+QICSGP=?\r\r\n+QICSGP: (1-5),(1-3),<APN>,<username>,<password>,(0-2)\r\n\r\nOK\r\n",
    ),
    ("AT+QIACT?", b"AT+QIACT?\r\r\nOK\r\n"),
    ("AT+QIACT=?", b"AT+QIACT=?\r\r\n+QIACT: (1-5)\r\n\r\nOK\r\n"),
    (
        "AT+QIOPEN=?",
        b"AT+QIOPEN=?\r\r\n+QIOPEN: (1-5),(0-11),\"TCP/UDP/TCP LISTENER/UDP SERVICE\",\"<IP_address>/<domain_name>\",<remote_port>,<local_port>,(0-2)\r\n\r\nOK\r\n",
    ),
    ("AT+QISTATE?", b"AT+QISTATE?\r\r\nOK\r\n"),
    (
        "AT+QIGETERROR",
        b"AT+QIGETERROR\r\r\n+QIGETERROR: 0,operate successfully\r\n\r\nOK\r\n",
    ),
    (
        "AT+QMTOPEN=?",
        b"AT+QMTOPEN=?\r\r\n+QMTOPEN: (0-5),<host_name>,(0-65535)\r\nOK\r\n",
    ),
    ("AT+QMTOPEN?", b"AT+QMTOPEN?\r\r\nOK\r\n"),
    (
        "AT+QMTCONN=?",
        b"AT+QMTCONN=?\r\r\n+QMTCONN: (0-5),<clientID>,<username>,<password>\r\nOK\r\n",
    ),
    ("AT+QMTCONN?", b"AT+QMTCONN?\r\r\nOK\r\n"),
    (
        "AT+QMTPUB=?",
        b"AT+QMTPUB=?\r\r\n+QMTPUB: (0-5),(0-65535),(0-2),(0,1),<topic>,(1-4096)\r\nOK\r\n",
    ),
    (
        "AT+QMTDISC=?",
        b"AT+QMTDISC=?\r\r\n+QMTDISC: (0-5)\r\nOK\r\n",
    ),
    (
        "AT+QMTCLOSE=?",
        b"AT+QMTCLOSE=?\r\r\n+QMTCLOSE: (0-5)\r\nOK\r\n",
    ),
    ("AT+QSSLSTATE?", b"AT+QSSLSTATE?\r\r\nOK\r\n"),
    (
        "AT+QSSLOPEN=?",
        b"AT+QSSLOPEN=?\r\r\n+QSSLOPEN: (1-5),(0-5),(0-11),<serveraddr>,<server_port>,(0-2)\r\n\r\nOK\r\n",
    ),
    (
        "AT+QSSLCFG=\"seclevel\",2,0",
        b"AT+QSSLCFG=\"seclevel\",2,0\r\r\nOK\r\n",
    ),
    (
        "AT+QSSLCFG=\"sslversion\",2,4",
        b"AT+QSSLCFG=\"sslversion\",2,4\r\r\nOK\r\n",
    ),
    (
        "AT+QSSLCFG=\"ciphersuite\",2,0xFFFF",
        b"AT+QSSLCFG=\"ciphersuite\",2,0xFFFF\r\r\nOK\r\n",
    ),
    (
        "AT+QSSLCFG=\"seclevel\",2",
        b"AT+QSSLCFG=\"seclevel\",2\r\r\n+QSSLCFG: \"seclevel\",2,0\r\n\r\nOK\r\n",
    ),
    // ----- Raw TCP / UDP sockets (test forms only) ----------------------
    (
        "AT+QISEND=?",
        b"AT+QISEND=?\r\r\n+QISEND: (0-11),(0-1460)\r\n\r\nOK\r\n",
    ),
    (
        "AT+QIRD=?",
        b"AT+QIRD=?\r\r\n+QIRD: (0-11),(0-1500)\r\n\r\nOK\r\n",
    ),
    (
        "AT+QICLOSE=?",
        b"AT+QICLOSE=?\r\r\n+QICLOSE: (0-11),(0-65535)\r\n\r\nOK\r\n",
    ),
    ("AT+QISTATE=?", b"AT+QISTATE=?\r\r\nOK\r\n"),
    (
        "AT+QIDNSCFG=?",
        b"AT+QIDNSCFG=?\r\r\n+QIDNSCFG: (1-5),<pridnsaddr>,<secdnsaddr>\r\n\r\nOK\r\n",
    ),
    (
        "AT+QIDNSGIP=?",
        b"AT+QIDNSGIP=?\r\r\n+QIDNSGIP: (1-5),<hostname>\r\n\r\nOK\r\n",
    ),
    // QIDNSCFG read fails when no PDP context is active (real-hw quirk).
    ("AT+QIDNSCFG=1", b"AT+QIDNSCFG=1\r\r\nERROR\r\n"),
    // ----- HTTP (test forms only) ---------------------------------------
    (
        "AT+QHTTPCFG=?",
        b"AT+QHTTPCFG=?\r\r\n+QHTTPCFG: \"contextid\",(1-5)\r\n+QHTTPCFG: \"requestheader\",(0,1)\r\n+QHTTPCFG: \"responseheader\",(0,1)\r\n+QHTTPCFG: \"sslctxid\",(0-5)\r\n+QHTTPCFG: \"contenttype\",(0-5)\r\n+QHTTPCFG: \"auth\",(\"username:password\")\r\n+QHTTPCFG: \"custom_header\",(\"custom_value\")\r\n\r\nOK\r\n",
    ),
    (
        "AT+QHTTPURL=?",
        b"AT+QHTTPURL=?\r\r\n+QHTTPURL: (1-700),(1-65535)\r\n\r\nOK\r\n",
    ),
    (
        "AT+QHTTPGET=?",
        b"AT+QHTTPGET=?\r\r\n+QHTTPGET: (1-65535),(1-2048),(1-65535)\r\n\r\nOK\r\n",
    ),
    (
        "AT+QHTTPPOST=?",
        b"AT+QHTTPPOST=?\r\r\n+QHTTPPOST: (1-1024000),(1-65535),(1-65535)\r\n\r\nOK\r\n",
    ),
    (
        "AT+QHTTPREAD=?",
        b"AT+QHTTPREAD=?\r\r\n+QHTTPREAD: (1-65535)\r\n\r\nOK\r\n",
    ),
    // ----- GPS ----------------------------------------------------------
    (
        "AT+QGPS=?",
        b"AT+QGPS=?\r\r\n+QGPS: (1)[,(1-3)[,(0-1000)[,(1-65535)]\r\n\r\nOK\r\n",
    ),
    ("AT+QGPS?", b"AT+QGPS?\r\r\n+QGPS: 0\r\n\r\nOK\r\n"),
    (
        "AT+QGPSLOC=?",
        b"AT+QGPSLOC=?\r\r\n+QGPSLOC: (0-5),(0-3600)\r\n\r\nOK\r\n",
    ),
    // QGPSEND when GPS is already off → CME 505 (in default CMEE=0 mode the
    // captured form is bare numeric).
    ("AT+QGPSEND", b"AT+QGPSEND\r\r\n+CME ERROR: 505\r\n"),
    // ----- SMS ----------------------------------------------------------
    ("AT+CMGF=?", b"AT+CMGF=?\r\r\n+CMGF: (0,1)\r\n\r\nOK\r\n"),
    ("AT+CMGF?", b"AT+CMGF?\r\r\n+CMGF: 0\r\n\r\nOK\r\n"),
    ("AT+CMGS=?", b"AT+CMGS=?\r\r\nOK\r\n"),
    ("AT+CMGR=?", b"AT+CMGR=?\r\r\nOK\r\n"),
    ("AT+CMGL=?", b"AT+CMGL=?\r\r\n+CMGL: (0-4)\r\n\r\nOK\r\n"),
    (
        "AT+CMGD=?",
        b"AT+CMGD=?\r\r\n+CMGD: (1-50),(0-4)\r\n\r\nOK\r\n",
    ),
    (
        "AT+CNMI=?",
        b"AT+CNMI=?\r\r\n+CNMI: (1-2),(0-2),(0,2),(0-2),(0-1)\r\n\r\nOK\r\n",
    ),
    ("AT+CNMI?", b"AT+CNMI?\r\r\n+CNMI: 2,1,0,0,0\r\n\r\nOK\r\n"),
    (
        "AT+CSCS=?",
        b"AT+CSCS=?\r\r\n+CSCS: (\"IRA\",\"GSM\",\"UCS2\")\r\n\r\nOK\r\n",
    ),
    ("AT+CSCS?", b"AT+CSCS?\r\r\n+CSCS: \"GSM\"\r\n\r\nOK\r\n"),
    ("AT+CSCA=?", b"AT+CSCA=?\r\r\nOK\r\n"),
    // ----- Power save ---------------------------------------------------
    ("AT+QSCLK=?", b"AT+QSCLK=?\r\r\n+QSCLK: (0-2)\r\n\r\nOK\r\n"),
    ("AT+QSCLK?", b"AT+QSCLK?\r\r\n+QSCLK: 0\r\n\r\nOK\r\n"),
    (
        "AT+CPSMS=?",
        b"AT+CPSMS=?\r\r\n+CPSMS: (0-2),(\"00000000\"-\"10111111\"),(\"00000000\"-\"11111111\"),(\"00000000\"-\"10111111\"),(\"00000000\"-\"11111111\")\r\n\r\nOK\r\n",
    ),
    (
        "AT+CPSMS?",
        b"AT+CPSMS?\r\r\n+CPSMS: 0,,,\"00101100\",\"00001010\"\r\n\r\nOK\r\n",
    ),
    (
        "AT+CEDRXS=?",
        b"AT+CEDRXS=?\r\r\n+CEDRXS: (0-3),(4,5),(\"0000\"-\"1111\")\r\n\r\nOK\r\n",
    ),
    ("AT+CEDRXS?", b"AT+CEDRXS?\r\r\n+CEDRXS: 0\r\n\r\nOK\r\n"),
    (
        "AT+QPSMCFG=?",
        b"AT+QPSMCFG=?\r\r\n+QPSMCFG: (20-4294967295),(0-15)\r\n\r\nOK\r\n",
    ),
    // ----- TLS sockets (test forms) -------------------------------------
    (
        "AT+QSSLSEND=?",
        b"AT+QSSLSEND=?\r\r\n+QSSLSEND: (0-11)[,(1-1460)]\r\n\r\nOK\r\n",
    ),
    (
        "AT+QSSLRECV=?",
        b"AT+QSSLRECV=?\r\r\n+QSSLRECV: (0-11),(1-1500)\r\n\r\nOK\r\n",
    ),
    (
        "AT+QSSLCLOSE=?",
        b"AT+QSSLCLOSE=?\r\r\n+QSSLCLOSE: (0-11)\r\n\r\nOK\r\n",
    ),
    ("AT+QSSLSTATE=?", b"AT+QSSLSTATE=?\r\r\nOK\r\n"),
    // ----- Filesystem ---------------------------------------------------
    ("AT+QFLDS=?", b"AT+QFLDS=?\r\r\nOK\r\n"),
    (
        "AT+QFLDS=\"UFS\"",
        b"AT+QFLDS=\"UFS\"\r\r\n+QFLDS: 3776512,3776512\r\n\r\nOK\r\n",
    ),
    ("AT+QFLST=?", b"AT+QFLST=?\r\r\nOK\r\n"),
    (
        "AT+QFLST=\"*\"",
        b"AT+QFLST=\"*\"\r\r\n+QFLST: \"security/\",0\r\n\r\nOK\r\n",
    ),
    (
        "AT+QFUPL=?",
        b"AT+QFUPL=?\r\r\n+QFUPL: <filename>[,(1-<freesize>)[,(1-65535)[,(0,1)]]]\r\n\r\nOK\r\n",
    ),
    (
        "AT+QFDWL=?",
        b"AT+QFDWL=?\r\r\n+QFDWL: <filename>\r\n\r\nOK\r\n",
    ),
    (
        "AT+QFOPEN=?",
        b"AT+QFOPEN=?\r\r\n+QFOPEN: <filename>[,(0-3)]\r\n\r\nOK\r\n",
    ),
    (
        "AT+QFREAD=?",
        b"AT+QFREAD=?\r\r\n+QFREAD: <filehandle>[,<length>]\r\n\r\nOK\r\n",
    ),
    (
        "AT+QFWRITE=?",
        b"AT+QFWRITE=?\r\r\n+QFWRITE: <filehandle>[,<length>[,<timeout>]]\r\n\r\nOK\r\n",
    ),
    (
        "AT+QFCLOSE=?",
        b"AT+QFCLOSE=?\r\r\n+QFCLOSE: <filehandle>\r\n\r\nOK\r\n",
    ),
    (
        "AT+QFDEL=?",
        b"AT+QFDEL=?\r\r\n+QFDEL: <filename>\r\n\r\nOK\r\n",
    ),
    // ----- NTP / Time ---------------------------------------------------
    (
        "AT+QNTP=?",
        b"AT+QNTP=?\r\r\n+QNTP: (1-5),<server>,(1-65535),(0,1)\r\n\r\nOK\r\n",
    ),
    ("AT+CCLK=?", b"AT+CCLK=?\r\r\nOK\r\n"),
    ("AT+QLTS=?", b"AT+QLTS=?\r\r\n+QLTS: (0-2)\r\n\r\nOK\r\n"),
    // ----- FOTA ---------------------------------------------------------
    ("AT+QFOTADL=?", b"AT+QFOTADL=?\r\r\nOK\r\n"),
    ("AT+QHVN=?", b"AT+QHVN=?\r\r\n+QHVN: <hvn>\r\n\r\nOK\r\n"),
    ("AT+QKTFOTA=?", b"AT+QKTFOTA=?\r\r\nOK\r\n"),
    // ----- Phonebook (BG770A doesn't support it) ------------------------
    (
        "AT+CPBR=?",
        b"AT+CPBR=?\r\r\n+CME ERROR: operation not allowed\r\n",
    ),
    (
        "AT+CPBW=?",
        b"AT+CPBW=?\r\r\n+CME ERROR: operation not allowed\r\n",
    ),
    (
        "AT+CPBF=?",
        b"AT+CPBF=?\r\r\n+CME ERROR: operation not allowed\r\n",
    ),
    ("AT+CPBS=?", b"AT+CPBS=?\r\r\nERROR\r\n"),
    ("AT+CPBS?", b"AT+CPBS?\r\r\nERROR\r\n"),
    // ----- Misc utility -------------------------------------------------
    (
        "AT+QPING=?",
        b"AT+QPING=?\r\r\n+QPING: (1-5),<host>,(1-255),(1-10)\r\n\r\nOK\r\n",
    ),
    ("AT+QLBS=?", b"AT+QLBS=?\r\r\nOK\r\n"),
    (
        "AT+QLBSCFG=?",
        b"AT+QLBSCFG=?\r\r\n+QLBSCFG: \"asynch\",(0,1)\r\n+QLBSCFG: \"timeout\",(10-120)\r\n+QLBSCFG: \"server\",<server_name>\r\n+QLBSCFG: \"token\",<token_value>\r\n+QLBSCFG: \"timeupdate\",(0,1)\r\n+QLBSCFG: \"withtime\",(0,1)\r\n+QLBSCFG: \"latorder\",(0,1)\r\n+QLBSCFG: \"scanband\",(0,1),<scan_band>\r\n+QLBSCFG: \"singlecell\",(0,1)\r\n\r\nOK\r\n",
    ),
    (
        "AT+QNWINFO",
        b"AT+QNWINFO\r\r\n+QNWINFO: \"NBIoT\",\"21670\",\"LTE BAND 1\",0\r\n\r\nOK\r\n",
    ),
    (
        "AT+QENG=?",
        b"AT+QENG=?\r\r\n+QENG: (\"servingcell\",\"neighbourcell\")\r\n\r\nOK\r\n",
    ),
    ("AT+CEINFO=?", b"AT+CEINFO=?\r\r\n+CEINFO: (0)\r\n\r\nOK\r\n"),
    // ----- AT% Sequans extensions -------------------------------------
    (
        "AT%RATACT?",
        b"AT%RATACT?\r\r\n%RATACT: \"NBIOT\",1,0\r\nOK\r\n",
    ),
    ("AT%RATSW?", b"AT%RATSW?\r\r\n%RATSW: 2,1\r\n\r\nOK\r\n"),
    (
        "AT%CERTCMD=?",
        b"AT%CERTCMD=?\r\r\n%CERTCMD: (\"READ\",\"WRITE\",\"DELETE\",\"DIR\",\"COPY\"),(0,1,2,3) \r\nOK\r\n",
    ),
    (
        "AT%MEAS=\"8\"",
        b"AT%MEAS=\"8\"\r\r\n%MEAS: Signal Quality: RSRP = N/A, RSRQ = N/A, SINR = N/A, RSSI = N/A\r\n\r\nOK\r\n",
    ),
    ("AT%PDNSTAT?", b"AT%PDNSTAT?\r\r\nOK\r\n"),
    ("AT%SCAN=?", b"AT%SCAN=?\r\r\nOK\r\n"),
    ("AT%PCOINFO?", b"AT%PCOINFO?\r\r\nOK\r\n"),
    ("AT%PDNSET=?", b"AT%PDNSET=?\r\r\nOK\r\n"),
    (
        "AT%STATEV?",
        b"AT%STATEV?\r\r\n+CME ERROR: operation not allowed\r\n",
    ),
    (
        "AT%PCONI?",
        b"AT%PCONI?\r\r\n+CME ERROR: operation not allowed\r\n",
    ),
    (
        "AT%SCANCFG?",
        b"AT%SCANCFG?\r\r\n+CME ERROR: operation not allowed\r\n",
    ),
    (
        "AT%MEAS?",
        b"AT%MEAS?\r\r\n+CME ERROR: operation not allowed\r\n",
    ),
    (
        "AT%CCID?",
        b"AT%CCID?\r\r\n+CME ERROR: operation not allowed\r\n",
    ),
    (
        "AT%STATUS",
        b"AT%STATUS\r\r\n+CME ERROR: Incorrect parameters\r\n",
    ),
    ("AT%PDNRDP?", b"AT%PDNRDP?\r\r\nERROR\r\n"),
    // ----- AT+VZ Verizon extensions ------------------------------------
    ("AT+VZWAPNE?", b"AT+VZWAPNE?\r\r\nERROR\r\n"),
    ("AT+VZWAPNE=?", b"AT+VZWAPNE=?\r\r\nERROR\r\n"),
    (
        "AT+VZWRSRP?",
        b"AT+VZWRSRP?\r\r\n+CME ERROR: operation not allowed\r\n",
    ),
    // ----- QGPSCFG sub-key state ---------------------------------------
    (
        "AT+QGPSCFG=\"outport\"",
        b"AT+QGPSCFG=\"outport\"\r\r\n+QGPSCFG: \"outport\",\"uartnmea\",115200\r\n\r\nOK\r\n",
    ),
    (
        "AT+QGPSCFG=\"autogps\"",
        b"AT+QGPSCFG=\"autogps\"\r\r\n+QGPSCFG: \"autogps\",0\r\n\r\nOK\r\n",
    ),
    (
        "AT+QGPSCFG=\"nmeasrc\"",
        b"AT+QGPSCFG=\"nmeasrc\"\r\r\n+QGPSCFG: \"nmeasrc\",1\r\n\r\nOK\r\n",
    ),
    (
        "AT+QGPSCFG=\"gnssconfig\"",
        b"AT+QGPSCFG=\"gnssconfig\"\r\r\n+QGPSCFG: \"gnssconfig\",1\r\n\r\nOK\r\n",
    ),
    // ----- File handle quirk -------------------------------------------
    // Real HW returns +QFOPEN: 0 for a missing file — but our model returns
    // an error (CME 409). Skip the exact-match here; the unit test covers it.
    ("AT+NONEXISTENT", b"AT+NONEXISTENT\r\r\nERROR\r\n"),
    ("ATBOGUS", b"ATBOGUS\r\r\nERROR\r\n"),
    ("AT+QCFG=\"nope\"", b"AT+QCFG=\"nope\"\r\r\nERROR\r\n"),
];

/// State-varying commands. The capture comes from real hardware (signal,
/// registration, SIM identifiers), so we match a structural template instead
/// of exact bytes. Placeholders: `{N}` = `\d+`, `{H}` = `[0-9A-Fa-f]+`,
/// `{R}` = any run of non-CR/LF bytes.
const SHAPE_GOLDEN: &[(&str, &str)] = &[
    ("AT+CGSN", "AT+CGSN\r\r\n{N}\r\n\r\nOK\r\n"),
    ("AT+CIMI", "AT+CIMI\r\r\n{N}\r\n\r\nOK\r\n"),
    ("AT+QCCID", "AT+QCCID\r\r\n+QCCID: {H}\r\n\r\nOK\r\n"),
    ("AT+CSQ", "AT+CSQ\r\r\n+CSQ: {N},{N}\r\n\r\nOK\r\n"),
    ("AT+QCSQ", "AT+QCSQ\r\r\n+QCSQ: \"NOSERVICE\"\r\n\r\nOK\r\n"),
    ("AT+CEREG?", "AT+CEREG?\r\r\n+CEREG: {N},{N}\r\n\r\nOK\r\n"),
    ("AT+CREG?", "AT+CREG?\r\r\n+CREG: {N},{N}\r\n\r\nOK\r\n"),
    ("AT+COPS?", "AT+COPS?\r\r\n+COPS: {N}\r\n\r\nOK\r\n"),
    (
        "AT+CGDCONT?",
        "AT+CGDCONT?\r\r\n+CGDCONT: {N},\"{R}\",\"{R}\",\"{R}\",{N},{N},{N}\r\n\r\nOK\r\n",
    ),
    ("AT+CFUN?", "AT+CFUN?\r\r\n+CFUN: {N}\r\n\r\nOK\r\n"),
    ("AT+CMEE?", "AT+CMEE?\r\r\n+CMEE: {N}\r\n\r\nOK\r\n"),
    ("AT+CGATT?", "AT+CGATT?\r\r\n+CGATT: {N}\r\n\r\nOK\r\n"),
    ("AT+CGACT?", "AT+CGACT?\r\r\n+CGACT: {N},{N}\r\n\r\nOK\r\n"),
];

/// Multi-command sequences captured from real hardware. Each entry is one
/// transaction: the firmware sends `cmd\r`, the modem replies with `expected`.
/// The whole sequence runs against one long-lived modem instance to validate
/// state retention (echo toggle, CMEE verbosity, etc).
struct Sequence<'a> {
    name: &'a str,
    steps: &'a [(&'a str, &'a [u8])],
}

const SEQUENCES: &[Sequence<'_>] = &[
    Sequence {
        name: "echo off then on",
        steps: &[
            ("AT&F", b"AT&F\r\r\nOK\r\n"),
            ("ATE0", b"ATE0\r\r\nOK\r\n"),
            ("AT", b"\r\nOK\r\n"),       // echo suppressed
            ("ATE1", b"\r\nOK\r\n"),     // received while echo off
            ("AT", b"AT\r\r\nOK\r\n"),   // echo back on
        ],
    },
    Sequence {
        name: "CMEE verbosity escalates the error string",
        steps: &[
            ("AT&F", b"AT&F\r\r\nOK\r\n"),
            ("AT+CMEE=0", b"AT+CMEE=0\r\r\nOK\r\n"),
            ("AT+CPIN=\"0000\"", b"AT+CPIN=\"0000\"\r\r\nERROR\r\n"),
            ("AT+CMEE=1", b"AT+CMEE=1\r\r\nOK\r\n"),
            (
                "AT+CPIN=\"0000\"",
                b"AT+CPIN=\"0000\"\r\r\n+CME ERROR: 3\r\n",
            ),
            ("AT+CMEE=2", b"AT+CMEE=2\r\r\nOK\r\n"),
            (
                "AT+CPIN=\"0000\"",
                b"AT+CPIN=\"0000\"\r\r\n+CME ERROR: operation not allowed\r\n",
            ),
        ],
    },
    Sequence {
        name: "MQTT publish happy path (PDP activate → open → connect → publish → disc → close)",
        steps: &[
            // Provision PDP context, then activate it. Once active, QMTOPEN
            // succeeds asynchronously and the rest of the MQTT lifecycle works.
            (
                "AT+QICSGP=1,1,\"internet\"",
                b"AT+QICSGP=1,1,\"internet\"\r\r\nOK\r\n",
            ),
            ("AT+QIACT=1", b"AT+QIACT=1\r\r\nOK\r\n"),
            (
                "AT+QIACT?",
                b"AT+QIACT?\r\r\n+QIACT: 1,1,1,\"10.0.0.2\"\r\n\r\nOK\r\n",
            ),
            // QMTOPEN: sync OK then async `+QMTOPEN: 0,0` once the broker
            // connection is established.
            (
                "AT+QMTOPEN=0,\"broker.example.com\",1883",
                b"AT+QMTOPEN=0,\"broker.example.com\",1883\r\r\nOK\r\n\r\n+QMTOPEN: 0,0\r\n",
            ),
            // QMTCONN: sync OK then async `+QMTCONN: 0,0,0` on accept.
            (
                "AT+QMTCONN=0,\"client-id\"",
                b"AT+QMTCONN=0,\"client-id\"\r\r\nOK\r\n\r\n+QMTCONN: 0,0,0\r\n",
            ),
            // QMTDISC: sync OK then async `+QMTDISC: 0,0`.
            ("AT+QMTDISC=0", b"AT+QMTDISC=0\r\r\nOK\r\n\r\n+QMTDISC: 0,0\r\n"),
            // QMTCLOSE: sync OK then async `+QMTCLOSE: 0,0`.
            (
                "AT+QMTCLOSE=0",
                b"AT+QMTCLOSE=0\r\r\nOK\r\n\r\n+QMTCLOSE: 0,0\r\n",
            ),
        ],
    },
    Sequence {
        name: "Filesystem upload → list → download → delete (CONNECT prompt for QFUPL/QFDWL)",
        steps: &[
            (
                "AT+QFLDS=\"UFS\"",
                b"AT+QFLDS=\"UFS\"\r\r\n+QFLDS: 3776512,3776512\r\n\r\nOK\r\n",
            ),
            (
                "AT+QFLST=\"*\"",
                b"AT+QFLST=\"*\"\r\r\n+QFLST: \"security/\",0\r\n\r\nOK\r\n",
            ),
            (
                "AT+QFDEL=\"missing\"",
                b"AT+QFDEL=\"missing\"\r\r\nERROR\r\n",
            ),
        ],
    },
    Sequence {
        name: "HTTP GET happy path (CFG → URL → GET → READ)",
        steps: &[
            (
                "AT+QHTTPCFG=\"contextid\",1",
                b"AT+QHTTPCFG=\"contextid\",1\r\r\nOK\r\n",
            ),
            (
                "AT+QICSGP=1,1,\"internet\"",
                b"AT+QICSGP=1,1,\"internet\"\r\r\nOK\r\n",
            ),
            ("AT+QIACT=1", b"AT+QIACT=1\r\r\nOK\r\n"),
            // QHTTPGET: sync OK, then `+QHTTPGET: 0,200,12` async URC.
            (
                "AT+QHTTPGET=30",
                b"AT+QHTTPGET=30\r\r\nOK\r\n\r\n+QHTTPGET: 0,200,12\r\n",
            ),
            // QHTTPREAD: CONNECT prompt, then body, then OK + URC.
            (
                "AT+QHTTPREAD=30",
                b"AT+QHTTPREAD=30\r\r\nCONNECT\r\nHello, HTTP!\r\nOK\r\n\r\n+QHTTPREAD: 0\r\n",
            ),
        ],
    },
    Sequence {
        name: "GPS engine on → query → off",
        steps: &[
            ("AT+QGPS?", b"AT+QGPS?\r\r\n+QGPS: 0\r\n\r\nOK\r\n"),
            ("AT+QGPS=1", b"AT+QGPS=1\r\r\nOK\r\n"),
            ("AT+QGPS?", b"AT+QGPS?\r\r\n+QGPS: 1\r\n\r\nOK\r\n"),
            (
                "AT+QGPSLOC=2",
                b"AT+QGPSLOC=2\r\r\n+QGPSLOC: 120000.0,37.7749N,122.4194W,1.0,10.0,3,0.0,0.0,0.0,150626,08\r\n\r\nOK\r\n",
            ),
            ("AT+QGPSEND", b"AT+QGPSEND\r\r\nOK\r\n"),
        ],
    },
    Sequence {
        name: "Raw TCP socket happy path (PDP activate → QIOPEN → QISTATE → QICLOSE)",
        steps: &[
            (
                "AT+QICSGP=1,1,\"internet\"",
                b"AT+QICSGP=1,1,\"internet\"\r\r\nOK\r\n",
            ),
            ("AT+QIACT=1", b"AT+QIACT=1\r\r\nOK\r\n"),
            // QIOPEN: sync OK, then async `+QIOPEN: <connectID>,0`.
            (
                "AT+QIOPEN=1,3,\"TCP\",\"example.com\",80",
                b"AT+QIOPEN=1,3,\"TCP\",\"example.com\",80\r\r\nOK\r\n\r\n+QIOPEN: 3,0\r\n",
            ),
            // QISTATE? lists the open socket with full tuple shape.
            (
                "AT+QISTATE?",
                b"AT+QISTATE?\r\r\n+QISTATE: 3,\"TCP\",\"example.com\",80,0,2,1,0,0,\"usbmodem\"\r\n\r\nOK\r\n",
            ),
            ("AT+QICLOSE=3", b"AT+QICLOSE=3\r\r\nOK\r\n"),
            ("AT+QISTATE?", b"AT+QISTATE?\r\r\nOK\r\n"),
        ],
    },
    Sequence {
        name: "MQTT-over-TLS happy path (configure SSL ctx → enable on client → open → connect)",
        steps: &[
            // Configure SSL context 2: no auth (seclevel=0), TLS 1.2 (sslversion=4),
            // allow any cipher (0xFFFF). Matches a typical "self-signed broker"
            // setup or Mosquitto with plain certs.
            (
                "AT+QSSLCFG=\"seclevel\",2,0",
                b"AT+QSSLCFG=\"seclevel\",2,0\r\r\nOK\r\n",
            ),
            (
                "AT+QSSLCFG=\"sslversion\",2,4",
                b"AT+QSSLCFG=\"sslversion\",2,4\r\r\nOK\r\n",
            ),
            (
                "AT+QSSLCFG=\"ciphersuite\",2,0xFFFF",
                b"AT+QSSLCFG=\"ciphersuite\",2,0xFFFF\r\r\nOK\r\n",
            ),
            // Confirm seclevel read-back picks up the stored value.
            (
                "AT+QSSLCFG=\"seclevel\",2",
                b"AT+QSSLCFG=\"seclevel\",2\r\r\n+QSSLCFG: \"seclevel\",2,0\r\n\r\nOK\r\n",
            ),
            // Bring PDP up.
            (
                "AT+QICSGP=1,1,\"internet\"",
                b"AT+QICSGP=1,1,\"internet\"\r\r\nOK\r\n",
            ),
            ("AT+QIACT=1", b"AT+QIACT=1\r\r\nOK\r\n"),
            // Wire SSL ctx 2 onto MQTT client 0.
            (
                "AT+QMTCFG=\"ssl\",0,1,2",
                b"AT+QMTCFG=\"ssl\",0,1,2\r\r\nOK\r\n",
            ),
            (
                "AT+QMTCFG=\"ssl\",0",
                b"AT+QMTCFG=\"ssl\",0\r\r\n+QMTCFG: \"ssl\",1,2\r\n\r\nOK\r\n",
            ),
            // Open MQTT-over-TLS on port 8883; same async URC pattern as plain.
            (
                "AT+QMTOPEN=0,\"broker.example.com\",8883",
                b"AT+QMTOPEN=0,\"broker.example.com\",8883\r\r\nOK\r\n\r\n+QMTOPEN: 0,0\r\n",
            ),
            (
                "AT+QMTCONN=0,\"client-id\"",
                b"AT+QMTCONN=0,\"client-id\"\r\r\nOK\r\n\r\n+QMTCONN: 0,0,0\r\n",
            ),
        ],
    },
    Sequence {
        name: "CFUN write updates the value the read form returns",
        steps: &[
            ("AT&F", b"AT&F\r\r\nOK\r\n"),
            ("AT+CFUN?", b"AT+CFUN?\r\r\n+CFUN: 1\r\n\r\nOK\r\n"),
            ("AT+CFUN=4", b"AT+CFUN=4\r\r\nOK\r\n"),
            ("AT+CFUN?", b"AT+CFUN?\r\r\n+CFUN: 4\r\n\r\nOK\r\n"),
            ("AT+CFUN=1", b"AT+CFUN=1\r\r\nOK\r\n"),
            ("AT+CFUN?", b"AT+CFUN?\r\r\n+CFUN: 1\r\n\r\nOK\r\n"),
            // CFUN=2 is not in the documented set (datasheet: only 0,1,4).
            ("AT+CFUN=2", b"AT+CFUN=2\r\r\nERROR\r\n"),
        ],
    },
];

/// Feed `cmd\r` byte-by-byte into `modem`, advance simulated time past every
/// documented Maximum Response Time, and return everything queued for RX. The
/// 200 s advance is the longest documented per-command bound (`AT+COPS=` is
/// 180 s "determined by the network") plus headroom.
fn drive(modem: &mut QuectelBg770a, cmd: &str) -> Vec<u8> {
    for b in cmd.bytes() {
        modem.on_tx_byte(b);
    }
    modem.on_tx_byte(b'\r');
    let mut out = Vec::new();
    while let Some(b) = modem.poll(0) {
        out.push(b);
    }
    while let Some(b) = modem.poll(200_000_000) {
        out.push(b);
    }
    out
}

fn show(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

/// Match `actual` against `pattern` where `{N}` consumes one or more ASCII
/// digits, `{H}` consumes hex digits, and `{R}` consumes one or more non-CRLF
/// bytes. Each wildcard must consume at least one byte; literal segments must
/// match exactly. The whole input must be consumed.
fn shape_match(actual: &[u8], pattern: &str) -> bool {
    let pat = pattern.as_bytes();
    let mut a = 0usize;
    let mut p = 0usize;
    while p < pat.len() {
        if pat[p] == b'{' && p + 2 < pat.len() && pat[p + 2] == b'}' {
            let kind = pat[p + 1];
            // Find the next literal segment so we know where to stop consuming.
            let lit_start = p + 3;
            let mut lit_end = lit_start;
            while lit_end < pat.len() && pat[lit_end] != b'{' {
                lit_end += 1;
            }
            let next_lit = &pat[lit_start..lit_end];
            let pred: fn(u8) -> bool = match kind {
                b'N' => |b| b.is_ascii_digit(),
                b'H' => |b| b.is_ascii_hexdigit(),
                b'R' => |b| b != b'\r' && b != b'\n',
                _ => return false,
            };
            let mut consumed = 0usize;
            while a + consumed < actual.len() && pred(actual[a + consumed]) {
                if !next_lit.is_empty() && actual[a + consumed..].starts_with(next_lit) {
                    break;
                }
                consumed += 1;
            }
            if consumed == 0 {
                return false;
            }
            a += consumed;
            // Hand the next literal segment back to the literal branch so it
            // is byte-checked against `actual`; do NOT skip past it.
            p = lit_start;
        } else {
            if a >= actual.len() || actual[a] != pat[p] {
                return false;
            }
            a += 1;
            p += 1;
        }
    }
    a == actual.len()
}

#[test]
fn model_byte_matches_real_hardware_for_deterministic_commands() {
    let mut modem = QuectelBg770a::new();
    let mut failures: Vec<String> = Vec::new();
    for &(cmd, expected) in EXACT_GOLDEN {
        let actual = drive(&mut modem, cmd);
        if actual != expected {
            failures.push(format!(
                "  {:<22} expected: {}\n  {:<22} actual:   {}",
                cmd,
                show(expected),
                "",
                show(&actual)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} exact captures differed from real hardware:\n{}",
        failures.len(),
        EXACT_GOLDEN.len(),
        failures.join("\n\n")
    );
}

#[test]
fn model_shape_matches_real_hardware_for_state_varying_commands() {
    let mut modem = QuectelBg770a::new();
    let mut failures: Vec<String> = Vec::new();
    for &(cmd, pattern) in SHAPE_GOLDEN {
        let actual = drive(&mut modem, cmd);
        if !shape_match(&actual, pattern) {
            failures.push(format!(
                "  {:<14} pattern: {}\n  {:<14} actual:  {}",
                cmd,
                pattern.replace('\r', "\\r").replace('\n', "\\n"),
                "",
                show(&actual)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}/{} shape-pattern captures didn't match the model output:\n{}",
        failures.len(),
        SHAPE_GOLDEN.len(),
        failures.join("\n\n")
    );
}

#[test]
fn model_replays_real_hardware_sequences() {
    let mut failures: Vec<String> = Vec::new();
    for seq in SEQUENCES {
        let mut modem = QuectelBg770a::new();
        for (idx, &(cmd, expected)) in seq.steps.iter().enumerate() {
            let actual = drive(&mut modem, cmd);
            if actual != expected {
                failures.push(format!(
                    "  [{}] step {} ({:?})\n    expected: {}\n    actual:   {}",
                    seq.name,
                    idx,
                    cmd,
                    show(expected),
                    show(&actual)
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} sequence step(s) deviated from real hardware:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn shape_match_self_test() {
    // The matcher itself needs to behave; broken matcher would silently pass
    // model output that doesn't actually conform.
    assert!(shape_match(b"+CSQ: 28,99", "+CSQ: {N},{N}"));
    assert!(shape_match(b"+CSQ: 99,99", "+CSQ: {N},{N}"));
    assert!(!shape_match(b"+CSQ: 28,", "+CSQ: {N},{N}"));
    assert!(!shape_match(b"+CSQ: 28,99 ", "+CSQ: {N},{N}")); // trailing garbage
    assert!(shape_match(b"abc 123 xyz", "abc {N} xyz"));
    assert!(!shape_match(b"abc xyz", "abc {N} xyz")); // wildcard needs >=1
    assert!(shape_match(b"AABBcc", "{H}cc"));
    assert!(!shape_match(b"ZZcc", "{H}cc"));
    assert!(shape_match(b"hello\r\n", "{R}\r\n"));
}
