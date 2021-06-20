//! [`Archive`](crate::Archive) implementations for `indexmap` types.

use crate::{
    collections::hash_index::{ArchivedHashIndex, HashIndexResolver},
    ser::Serializer,
    Archive,
    Archived,
    Deserialize,
    Fallible,
    RelPtr,
    Serialize,
};
use core::{
    borrow::Borrow,
    hash::Hash,
    iter::FusedIterator,
    marker::PhantomData,
    mem::MaybeUninit,
};
use indexmap::IndexMap;

#[cfg_attr(feature = "strict", repr(C))]
struct Entry<K, V> {
    key: K,
    value: V,
}

impl<K: Archive, V: Archive> Archive for Entry<&'_ K, &'_ V> {
    type Archived = Entry<K::Archived, V::Archived>;
    type Resolver = (K::Resolver, V::Resolver);

    #[inline]
    unsafe fn resolve(
        &self,
        pos: usize,
        resolver: Self::Resolver,
        out: &mut MaybeUninit<Self::Archived>,
    ) {
        let (fp, fo) = out_field!(out.key);
        self.key.resolve(pos + fp, resolver.0, fo);

        let (fp, fo) = out_field!(out.value);
        self.value.resolve(pos + fp, resolver.1, fo);
    }
}

/// An archived `IndexMap`.
#[cfg_attr(feature = "strict", repr(C))]
pub struct ArchivedIndexMap<K, V> {
    index: ArchivedHashIndex,
    pivots: RelPtr<Archived<usize>>,
    entries: RelPtr<Entry<K, V>>,
}

impl<K, V> ArchivedIndexMap<K, V> {
    #[inline]
    unsafe fn pivot(&self, index: usize) -> usize {
        *self.pivots.as_ptr().add(index) as usize
    }

    #[inline]
    unsafe fn entry(&self, index: usize) -> &Entry<K, V> {
        &*self.entries.as_ptr().add(index)
    }

    #[inline]
    fn find<Q: ?Sized>(&self, k: &Q) -> Option<usize>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.index.index(k)
            .and_then(|pivot_index| {
                let index = unsafe { self.pivot(pivot_index) };
                let entry = unsafe { self.entry(index) };
                if entry.key.borrow() == k {
                    Some(index)
                } else {
                    None
                }
            })
    }

    /// Returns whether a key is present in the hash map.
    #[inline]
    pub fn contains_key<Q: ?Sized>(&self, k: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.find(k).is_some()
    }

    /// Returns the first key-value pair.
    #[inline]
    pub fn first(&self) -> Option<(&K, &V)> {
        if self.len() > 0 {
            let entry = unsafe { self.entry(0) };
            Some((&entry.key, &entry.value))
        } else {
            None
        }
    }

    /// Gets the value associated with the given key.
    #[inline]
    pub fn get<Q: ?Sized>(&self, k: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.find(k).map(|index| unsafe { &self.entry(index).value })
    }

    /// Gets the index, key, and value associated with the given key.
    #[inline]
    pub fn get_full<Q: ?Sized>(&self, k: &Q) -> Option<(usize, &K, &V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.find(k).map(|index| {
            let entry = unsafe { &self.entry(index) };
            (index, &entry.key, &entry.value)
        })
    }

    /// Gets a key-value pair by index.
    #[inline]
    pub fn get_index(&self, index: usize) -> Option<(&K, &V)> {
        if index < self.len() {
            let entry = unsafe { &self.entry(index) };
            Some((&entry.key, &entry.value))
        } else {
            None
        }
    }

    /// Gets the index of a key if it exists in the map.
    #[inline]
    pub fn get_index_of<Q: ?Sized>(&self, key: &Q) -> Option<usize>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.find(key).map(|index| index)
    }

    /// Gets the key-value pair associated with the given key.
    #[inline]
    pub fn get_key_value<Q: ?Sized>(&self, k: &Q) -> Option<(&K, &V)>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.find(k).map(|index| {
            let entry = unsafe { &self.entry(index) };
            (&entry.key, &entry.value)
        })
    }

    /// Gets the hasher for this index map.
    #[inline]
    pub fn hasher(&self) -> seahash::SeaHasher {
        self.index.hasher()
    }

    /// Returns `true` if the map contains no elements.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    fn raw_iter(&self) -> RawIter<K, V> {
        RawIter::new(self.entries.as_ptr().cast(), self.len())
    }

    /// Returns an iterator over the key-value pairs of the map in order
    #[inline]
    pub fn iter(&self) -> Iter<K, V> {
        Iter {
            inner: self.raw_iter(),
        }
    }

    /// Returns an iterator over the keys of the map in order
    #[inline]
    pub fn keys(&self) -> Keys<K, V> {
        Keys {
            inner: self.raw_iter(),
        }
    }

    /// Gets the number of items in the index map.
    #[inline]
    pub const fn len(&self) -> usize {
        self.index.len()
    }

    /// Returns an iterator over the values of the map in order
    #[inline]
    pub fn values(&self) -> Values<K, V> {
        Values {
            inner: self.raw_iter(),
        }
    }
}

impl<K: PartialEq, V: PartialEq> PartialEq for ArchivedIndexMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl<UK, K: PartialEq<UK>, UV, V: PartialEq<UV>> PartialEq<IndexMap<UK, UV>> for ArchivedIndexMap<K, V> {
    fn eq(&self, other: &IndexMap<UK, UV>) -> bool {
        self.iter().zip(other.iter()).all(|((ak, av), (bk, bv))| ak == bk && av == bv)
    }
}

struct RawIter<'a, K, V> {
    current: *const Entry<K, V>,
    remaining: usize,
    _phantom: PhantomData<(&'a K, &'a V)>,
}

impl<'a, K, V> RawIter<'a, K, V> {
    #[inline]
    fn new(pairs: *const Entry<K, V>, len: usize) -> Self {
        Self {
            current: pairs,
            remaining: len,
            _phantom: PhantomData,
        }
    }
}

impl<'a, K, V> Iterator for RawIter<'a, K, V> {
    type Item = (&'a K, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            if self.remaining == 0 {
                None
            } else {
                let result = self.current;
                self.current = self.current.add(1);
                self.remaining -= 1;
                let entry = &*result;
                Some((&entry.key, &entry.value))
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<'a, K, V> ExactSizeIterator for RawIter<'a, K, V> {}
impl<'a, K, V> FusedIterator for RawIter<'a, K, V> {}

/// An iterator over the key-value pairs of an index map.
#[repr(transparent)]
pub struct Iter<'a, K, V> {
    inner: RawIter<'a, K, V>,
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<K, V> ExactSizeIterator for Iter<'_, K, V> {}
impl<K, V> FusedIterator for Iter<'_, K, V> {}

/// An iterator over the keys of an index map.
#[repr(transparent)]
pub struct Keys<'a, K, V> {
    inner: RawIter<'a, K, V>,
}

impl<'a, K, V> Iterator for Keys<'a, K, V> {
    type Item = &'a K;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(k, _)| k)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<K, V> ExactSizeIterator for Keys<'_, K, V> {}
impl<K, V> FusedIterator for Keys<'_, K, V> {}

/// An iterator over the values of an index map.
#[repr(transparent)]
pub struct Values<'a, K, V> {
    inner: RawIter<'a, K, V>,
}

impl<'a, K, V> Iterator for Values<'a, K, V> {
    type Item = &'a V;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(_, v)| v)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<K, V> ExactSizeIterator for Values<'_, K, V> {}
impl<K, V> FusedIterator for Values<'_, K, V> {}

// Archive implementations

/// The resolver for an `IndexMap`.
pub struct IndexMapResolver {
    index_resolver: HashIndexResolver,
    pivots_pos: usize,
    entries_pos: usize,
}

impl<K: Archive, V: Archive> Archive for IndexMap<K, V> {
    type Archived = ArchivedIndexMap<K::Archived, V::Archived>;
    type Resolver = IndexMapResolver;

    unsafe fn resolve(
        &self,
        pos: usize,
        resolver: Self::Resolver,
        out: &mut MaybeUninit<Self::Archived>,
    ) {
        let (fp, fo) = out_field!(out.index);
        ArchivedHashIndex::resolve_from_len(self.len(), pos + fp, resolver.index_resolver, fo);

        let (fp, fo) = out_field!(out.pivots);
        RelPtr::emplace(pos + fp, resolver.pivots_pos, fo);

        let (fp, fo) = out_field!(out.entries);
        RelPtr::emplace(pos + fp, resolver.entries_pos, fo);
    }
}

impl<K: Hash + Eq + Serialize<S>, V: Serialize<S>, S: Serializer + ?Sized> Serialize<S> for IndexMap<K, V> {
    fn serialize(&self, serializer: &mut S) -> Result<IndexMapResolver, S::Error> {
        unsafe {
            let (index_resolver, entries) = ArchivedHashIndex::build_and_serialize(
                self.iter(),
                serializer,
            )?;

            // Serialize entries
            let mut resolvers = self
                .iter()
                .map(|(key, value)| Ok((key.serialize(serializer)?, value.serialize(serializer)?)))
                .collect::<Result<Vec<_>, _>>()?;

            let entries_pos = serializer.align_for::<Entry<K::Archived, V::Archived>>()?;
            for ((key, value), (key_resolver, value_resolver)) in
                self.iter().zip(resolvers.drain(..))
            {
                serializer.resolve_aligned(&Entry { key, value }, (key_resolver, value_resolver))?;
            }

            // Serialize pivots
            let pivots_pos = serializer.align_for::<Archived<usize>>()?;
            for &(key, _) in entries.iter() {
                serializer.resolve_aligned(&self.get_index_of(key).unwrap(), ())?;
            }

            Ok(IndexMapResolver {
                index_resolver,
                pivots_pos,
                entries_pos,
            })
        }
    }
}

impl<K, V, D> Deserialize<IndexMap<K, V>, D> for ArchivedIndexMap<K::Archived, V::Archived>
where
    K: Archive + Hash + Eq,
    K::Archived: Deserialize<K, D>,
    V: Archive,
    V::Archived: Deserialize<V, D>,
    D: Fallible + ?Sized,
{
    fn deserialize(&self, deserializer: &mut D) -> Result<IndexMap<K, V>, D::Error> {
        let mut result = IndexMap::with_capacity(self.len());
        for (k, v) in self.iter() {
            result.insert(k.deserialize(deserializer)?, v.deserialize(deserializer)?);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        archived_root,
        ser::{serializers::AlignedSerializer, Serializer},
        util::AlignedVec,
        Deserialize,
        Infallible,
    };
    use indexmap::{indexmap, IndexMap};

    #[test]
    fn index_map() {
        let value = indexmap! {
            String::from("foo") => 10,
            String::from("bar") => 20,
            String::from("baz") => 40,
            String::from("bat") => 80,
        };

        let mut serializer = AlignedSerializer::new(AlignedVec::new());
        serializer.serialize_value(&value).unwrap();
        let result = serializer.into_inner();
        let archived = unsafe { archived_root::<IndexMap<String, i32>>(result.as_ref()) };

        assert_eq!(value.len(), archived.len());
        for (k, v) in value.iter() {
            let (ak, av) = archived.get_key_value(k.as_str()).unwrap();
            assert_eq!(k, ak);
            assert_eq!(v, av);
        }

        let deserialized = Deserialize::<IndexMap<String, i32>, _>::deserialize(
            archived,
            &mut Infallible,
        ).unwrap();
        assert!(value == deserialized);
    }
}
