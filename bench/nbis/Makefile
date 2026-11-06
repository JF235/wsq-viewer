# Build the NBIS timing harness against an NBIS 5.0.0 build tree:
#   make NBIS=/path/to/Rel_5.0.0 OUT=/path/to/build/dir
ifndef NBIS
$(error set NBIS to the Rel_5.0.0 folder of an NBIS build)
endif
OUT ?= .
CFLAGS ?= -O2

$(OUT)/nbis_bench: nbis_bench.c
	gcc $(CFLAGS) -o $@ $< -I$(NBIS)/exports/include -L$(NBIS)/exports/lib \
	    -lwsq -ljpegl -lfet -lioutil -lutil -lm
