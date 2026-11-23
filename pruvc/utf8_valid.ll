
define internal i1 @pruv_utf8_valid(ptr %data, i64 %len) nounwind {
entry:
  br label %head

head:
  %i = phi i64 [ 0, %entry ], [ %next, %ascii ], [ %end, %continuation ], [ %resume, %block_exit ]
  %done = icmp eq i64 %i, %len
  br i1 %done, label %valid, label %lead

lead:
  %p = getelementptr inbounds i8, ptr %data, i64 %i
  %byte = load i8, ptr %p
  %is_ascii = icmp ult i8 %byte, 128
  %next = add i64 %i, 1
  br i1 %is_ascii, label %ascii, label %sequence

ascii:
  ; At a 16-byte aligned ASCII byte, skip whole 16-byte ASCII blocks (two aligned words with no high bit).
  %addr = ptrtoint ptr %p to i64
  %misaligned = and i64 %addr, 15
  %aligned = icmp eq i64 %misaligned, 0
  br i1 %aligned, label %block_head, label %head

block_head:
  %k = phi i64 [ %i, %ascii ], [ %k16, %block ]
  %left = sub i64 %len, %k
  %room = icmp uge i64 %left, 16
  br i1 %room, label %block, label %block_exit

block:
  %bp = getelementptr inbounds i8, ptr %data, i64 %k
  %w0 = load i64, ptr %bp, align 8
  %bp8 = getelementptr inbounds i8, ptr %bp, i64 8
  %w1 = load i64, ptr %bp8, align 8
  %w = or i64 %w0, %w1
  %high_bits = and i64 %w, -9187201950435737472
  %all_ascii = icmp eq i64 %high_bits, 0
  %k16 = add i64 %k, 16
  br i1 %all_ascii, label %block_head, label %block_exit

block_exit:
  ; %k is the first block not known to be ASCII; data[%i] is ASCII, so never resume before %next.
  %stop = phi i64 [ %k, %block_head ], [ %k, %block ]
  %same = icmp eq i64 %stop, %i
  %resume = select i1 %same, i64 %next, i64 %stop
  br label %head

sequence:
  ; Legal multibyte leads are C2..F4.
  %lead_offset = sub i8 %byte, 194
  %lead_ok = icmp ule i8 %lead_offset, 50
  %two = icmp ult i8 %byte, 224
  %three = icmp ult i8 %byte, 240
  %width34 = select i1 %three, i64 3, i64 4
  %width = select i1 %two, i64 2, i64 %width34
  %remaining = sub i64 %len, %i
  %fits = icmp uge i64 %remaining, %width
  %can_read = and i1 %lead_ok, %fits
  br i1 %can_read, label %first, label %invalid

first:
  ; Restrict the second byte for overlong encodings, surrogates and values above U+10FFFF.
  %e0 = icmp eq i8 %byte, 224
  %ed = icmp eq i8 %byte, 237
  %f0 = icmp eq i8 %byte, 240
  %f4 = icmp eq i8 %byte, 244
  %low_base = select i1 %f0, i8 144, i8 128
  %low = select i1 %e0, i8 160, i8 %low_base
  %high_base = select i1 %f4, i8 143, i8 191
  %high = select i1 %ed, i8 159, i8 %high_base
  %second_ptr = getelementptr inbounds i8, ptr %data, i64 %next
  %second = load i8, ptr %second_ptr
  %above_low = icmp uge i8 %second, %low
  %below_high = icmp ule i8 %second, %high
  %second_ok = and i1 %above_low, %below_high
  %after_second = add i64 %i, 2
  %end = add i64 %i, %width
  br i1 %second_ok, label %continuation, label %invalid

continuation:
  %j = phi i64 [ %after_second, %first ], [ %advance, %tail ]
  %complete = icmp eq i64 %j, %end
  br i1 %complete, label %head, label %tail

tail:
  %tail_ptr = getelementptr inbounds i8, ptr %data, i64 %j
  %rest = load i8, ptr %tail_ptr
  %tag = and i8 %rest, 192
  %tail_ok = icmp eq i8 %tag, 128
  %advance = add i64 %j, 1
  br i1 %tail_ok, label %continuation, label %invalid

valid:
  ret i1 true

invalid:
  ret i1 false
}
