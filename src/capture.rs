use core::{ops::{Add, Sub}, slice};
use std::ops::{AddAssign, Index, IndexMut, Range, RangeFrom, RangeFull, RangeTo, SubAssign};

use crate::Result;

#[derive(Debug)]
pub struct RingSlice {
    ptr: *mut u8,
    len: usize,
}

// SAFETY: Conceptually the same as `Box<[u8]>`. The destructor can run on any thread.
unsafe impl Send for RingSlice {}

impl RingSlice {
    pub fn new(min_size: usize) -> Result<RingSlice> {
        let len = min_size.next_multiple_of(vmap::allocation_size());
        // `pread()` gets unhappy if you read into the same page twice from both ends.
        let len = len.max(vmap::page_size() * 2);
        let ptr = vmap::os::map_ring(len)?;
        log::trace!("mapped ring slice at {:?}+{:#x?}*2", ptr, len);
        Ok(RingSlice { ptr, len })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.ptr
    }
}

impl Drop for RingSlice {
    fn drop(&mut self) {
        // SAFETY: Mapped with the same parameters in `Self::new`.
        let result = unsafe { vmap::os::unmap_ring(self.ptr, self.len) };
        result.expect("failed to unmap ring slice");
        log::trace!("unmapped ring slice at {:?}+{:#x?}*2", self.ptr, self.len);
    }
}

macro_rules! index_range {
    {
        fn range_to_parts(&any $self:ident, $index:ident: $range_ty:ty) { $( $code:tt )* }
        $( $rest:tt )*
    } => {
        impl Index<$range_ty> for RingSlice {
            type Output = [u8];

            fn index(&$self, $index: $range_ty) -> &Self::Output {
                unsafe {
                    let (ptr, len) = { $( $code )* };
                    slice::from_raw_parts(ptr, len)
                }
            }
        }

        impl IndexMut<$range_ty> for RingSlice {
            fn index_mut(&mut $self, $index: $range_ty) -> &mut Self::Output {
                unsafe {
                    let (ptr, len) = { $( $code )* };
                    slice::from_raw_parts_mut(ptr, len)
                }
            }
        }

        index_range! { $( $rest )* }
    };
    {} => {}
}

index_range! {
    fn range_to_parts(&any self, index: Range<usize>) {
        assert!(index.start < self.len && index.end <= self.len);
        if index.end >= index.start {
            (self.ptr.offset(index.start as isize), index.end - index.start)
        } else {
            (self.ptr.offset(index.start as isize), (self.len - index.start) + index.end)
        }
    }

    fn range_to_parts(&any self, index: RangeFrom<usize>) {
        assert!(index.start < self.len);
        (self.ptr.offset(index.start as isize), self.len)
    }

    // Perhaps counterintuitively, the same rotate operation as `Index<RangeFrom<usize>>`!
    fn range_to_parts(&any self, index: RangeTo<usize>) {
        assert!(index.end <= self.len);
        (self.ptr.offset(index.end as isize), self.len)
    }

    fn range_to_parts(&any self, _index: RangeFull) {
        (self.ptr, self.len)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingCursor {
    index: usize,
    bound: usize,
}

impl RingCursor {
    pub fn new(bound: usize) -> RingCursor {
        RingCursor { index: 0, bound }
    }

    pub fn into_inner(self) -> usize {
        self.index
    }
}

impl Add<usize> for RingCursor {
    type Output = RingCursor;

    fn add(self, offset: usize) -> Self::Output {
        RingCursor { index: self.index.wrapping_add(offset) % self.bound, bound: self.bound }
    }
}

impl AddAssign<usize> for RingCursor {
    fn add_assign(&mut self, offset: usize) {
        *self = *self + offset
    }
}

impl Sub<usize> for RingCursor {
    type Output = RingCursor;

    fn sub(self, offset: usize) -> Self::Output {
        RingCursor { index: self.index.wrapping_sub(offset) % self.bound, bound: self.bound }
    }
}

impl SubAssign<usize> for RingCursor {
    fn sub_assign(&mut self, offset: usize) {
        *self = *self - offset
    }
}

#[derive(Debug)]
pub struct RingBuffer {
    buffer: RingSlice,
    cursor: RingCursor,
}

impl RingBuffer {
    pub fn new(min_size: usize) -> Result<RingBuffer> {
        let buffer = RingSlice::new(min_size)?;
        let cursor = RingCursor::new(buffer.len());
        Ok(RingBuffer { buffer, cursor })
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn cursor(&self) -> RingCursor {
        self.cursor
    }

    pub fn append<F, E>(&mut self, max_size: usize, writer: F) -> core::result::Result<usize, E>
            where F: FnOnce(&mut [u8]) -> core::result::Result<usize, E> {
        assert!(max_size <= self.buffer.len());
        let result = writer(&mut self.buffer[self.cursor.index..][..max_size]);
        if let Ok(written) = result { self.cursor += written }
        result
    }

    pub fn read(&self, cursor: RingCursor, count: usize) -> &[i8] {
        assert!(cursor.bound == self.buffer.len() && count <= self.buffer.len());
        bytemuck::cast_slice(&self.buffer[cursor.index..][..count])
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_ring_buffer_simple() {
        let mut buf = RingSlice::new(8).unwrap();
        buf[..][0..8].copy_from_slice([1, 2, 3, 4, 5, 6, 7, 8].as_ref());
        assert_eq!(&buf[0..4], [1, 2, 3, 4].as_ref());
        assert_eq!(&buf[2..6], [3, 4, 5, 6].as_ref());
        assert_eq!(&buf[4..8], [5, 6, 7, 8].as_ref());
        assert_eq!(&buf[5..][..3], [6, 7, 8].as_ref());
        assert_eq!(&buf[..5][buf.len() - 5..], [1, 2, 3, 4, 5].as_ref());
    }

    #[test]
    fn test_ring_buffer_overlap() {
        let mut buf = RingSlice::new(8192).unwrap();
        assert_eq!(buf.len(), 8192);
        buf[8186..8192].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        buf[0..6].copy_from_slice(&[7, 8, 9, 10, 11, 12]);
        assert_eq!(&buf[8186..8192], &[1, 2, 3, 4, 5, 6]);
        assert_eq!(&buf[0..6], &[7, 8, 9, 10, 11, 12]);
        assert_eq!(&buf[8186..6], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }

    #[test]
    fn test_ring_cursor() {
        let cursor = RingCursor::new(128);
        assert_eq!((cursor + 10).index, 10);
        assert_eq!((cursor + 10 + 120).index, 2);
        assert_eq!((cursor + 130).index, 2);
        assert_eq!((cursor - 10).index, 118);
        assert_eq!((cursor - 130).index, 126);
        assert_eq!((cursor + 0), cursor);
        let mut cursor = cursor;
        cursor += 10;
        assert_eq!(cursor.index, 10);
        cursor -= 20;
        assert_eq!(cursor.index, 118);
    }
}
