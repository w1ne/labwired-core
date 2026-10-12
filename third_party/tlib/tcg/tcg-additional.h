#pragma once

void attach_gen_opc_buf(void *buf);
void attach_tcg(void *tcg_c);
void attach_code_gen_prologue(void *prol);
void attach_gen_opparam_buf(void *buf);
void attach_ld_helpers(void *__ldb, void *__ldw, void *__ldl, void *__ldq);
void attach_st_helpers(void *__stb, void *__stw, void *__stl, void *__stq);

void set_temp_buf_offset(unsigned int offset);
void set_tlb_table_n_0_rwa(int i, unsigned int read, unsigned int write, unsigned int addend);
void set_tlb_table_n_0(int i, unsigned int offset);
void set_TARGET_PAGE_BITS(int val);
void set_sizeof_CPUTLBEntry(unsigned int sz);
void set_tlb_entry_addr_rwu(unsigned int read, unsigned int write, unsigned int addend);

void attach_malloc(void *malloc_callback);
void attach_realloc(void *reall);
void attach_free(void *free_callback);
