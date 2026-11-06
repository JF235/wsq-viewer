#!/bin/bash
# Run every measurement of the performance document. Results go to $OUT as text tables.
#   WSQ=<wsq binary> NBIS_BENCH=<nbis_bench> REF=<folder with the NBIS-encoded *_0.75.wsq,
#   *_2.25.wsq and the PGMs> WASM=<folder with wsq_scalar.wasm, wsq_simd.wasm> OUT=<folder> run_all.sh
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT"
cd "$REF"
P=2               # one performance core for single-thread runs
NIST=$(ls *.pgm | sed 's/.pgm$//')
BACKENDS=$("$WSQ" backends | awk '/lanes/ {print $1}')   # the ones this processor has
{ lscpu | grep -E 'Model name'; rustc --version; node --version; gcc --version | head -1; date -Iseconds; } > "$OUT/machine.txt"

for rate in 0.75 2.25; do
  files=$(for n in $NIST; do echo -n "${n}_$rate.wsq "; done)
  taskset -c $P "$NBIS_BENCH" decode 9 $files > "$OUT/decode_nbis_$rate.txt" 2>/dev/null
  for b in $BACKENDS; do
    for m in exact fast; do
      flag=""; [ $m = fast ] && flag="--fast"
      taskset -c $P "$WSQ" bench decode $flag --backend $b --reps 9 $files > "$OUT/decode_${b}_${m}_$rate.txt"
    done
  done
  pgms=$(for n in $NIST; do echo -n "$n.pgm "; done)
  taskset -c $P "$NBIS_BENCH" encode 5 $rate $pgms > "$OUT/encode_nbis_$rate.txt"
  for b in $BACKENDS; do
    for m in exact fast; do
      flag=""; [ $m = fast ] && flag="--fast"
      taskset -c $P "$WSQ" bench encode $flag --backend $b --reps 5 --rate $rate $pgms > "$OUT/encode_${b}_${m}_$rate.txt"
    done
  done
done

: > "$OUT/threads.txt"
for t in 1 2 4 8; do
  taskset -c 0-7 "$WSQ" bench decode --backend avx2 --reps 15 --threads $t a002_0.75.wsq sample_19_0.75.wsq cmp00001_0.75.wsq | sed "s/^/threads=$t /" >> "$OUT/threads.txt"
done

: > "$OUT/batch.txt"
all=$(for n in $NIST; do echo -n "${n}_0.75.wsq "; done)
for t in 1 2 4 8 12; do
  echo "exact $("$WSQ" bench batch --backend avx2 --threads $t --rounds 10 $all)" >> "$OUT/batch.txt"
  echo "fast $("$WSQ" bench batch --fast --backend avx2 --threads $t --rounds 10 $all)" >> "$OUT/batch.txt"
done
echo "coefficients $("$WSQ" bench batch-coefficients --fast --backend avx2 --threads 4 --rounds 10 $all)" >> "$OUT/batch.txt"

: > "$OUT/wasm.txt"
for n in a002 cmp00001 sample_19; do
  for m in wsq_scalar wsq_simd; do
    taskset -c $P node "$HERE/wasm/bench.mjs" "$WASM/$m.wasm" 15 ${n}_0.75.wsq ${n}_0.75.raw >> "$OUT/wasm.txt" 2>&1 || true
  done
done
echo done > "$OUT/DONE"
