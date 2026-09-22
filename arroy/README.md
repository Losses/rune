<p align="center"><img width="280px" title="The arroy logo is electric like LMDB and made up of colored clusters (of vectors)" src="https://raw.githubusercontent.com/meilisearch/arroy/main/assets/arroy-electric-clusters-logo.png"></a>
<h1 align="center">arroy</h1>

[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)
[![Crates.io](https://img.shields.io/crates/v/arroy)](https://crates.io/crates/arroy)
[![Docs](https://docs.rs/arroy/badge.svg)](https://docs.rs/arroy)
[![dependency status](https://deps.rs/repo/github/meilisearch/arroy/status.svg)](https://deps.rs/repo/github/meilisearch/arroy)
[![Build](https://github.com/meilisearch/arroy/actions/workflows/rust.yml/badge.svg)](https://github.com/meilisearch/arroy/actions/workflows/rust.yml)

Arroy ([Approximate Rearest Reighbors][1] Oh Yeah) is a Rust library with an interface close of the [Annoy Python library][2] to search for vectors in space that are near a targeted vector. It is based on LMDB, a memory-mapped key-value store, so many processes may share the same data and atomically modify the vectors.

## Background

There are some other libraries to do nearest neighbor search. However, most of them are memory-bound, and none use LMDB for their storage. [Annoy considered using LMDB][3] as a backend since 2015. We built Meilisearch on top of LMDB; therefore, it was an obvious choice. As Annoy highly inspires it, we benefit from the same low memory footprint.

Why is this useful? If you want to find the nearest neighbors and have many CPUs, you only need to build the index once. Any thread will be able to query the LMDB-based index and will be able to do lookups immediately, even while another index is modifying it.

We use it inside [Meilisearch](https://github.com/meilisearch/meilisearch). This library helps our users search for similar documents. Our users have many millions of them in a high-dimensional space (i.e., 768 on average and 1536 for OpenAI), so memory usage is a prime concern.

Arroy was built by [@Kerollmops](https://github.com/Kerollmops) and [@irevoire](https://github.com/irevoire) with the help of [@dureuill](https://github.com/dureuill) in a week by porting the original C++ source code of Annoy.

## Summary of features

- [Euclidean distance](https://en.wikipedia.org/wiki/Euclidean_distance), [Manhattan distance](https://en.wikipedia.org/wiki/Taxicab_geometry), [cosine distance](https://en.wikipedia.org/wiki/Cosine_similarity), or [Dot (Inner) Product distance](https://en.wikipedia.org/wiki/Dot_product)
- Cosine distance is equivalent to Euclidean distance of normalized vectors i.e., `sqrt(2-2*cos(u, v))`
- Works better if you don't have too many dimensions (like <100) but seems to perform surprisingly well even up to 1,000 dimensions
- Small memory usage
- Lets you share memory between multiple processes using LMDB
- Index creation is separate from lookup (in particular, you can not add more items once the tree has been created)
- Build index on disk to enable indexing big datasets that won't fit into memory using LMDB
- Multithreaded tree building using rayon
- Additional features compared to Annoy
  - Filter when querying
  - Incrementally update the tree without rebuilding it from scratch
  - Store and modify different indexes atomically using LMDB (indexes are identified by an `u16`)
  - Modify the items list **in place** while performing queries using LMDB
  - Storage based on LMDB using LMDB
  - Safer to use API, i.e., check dimensions, distances, etc
  - The database size does not depend on the highest item ID but on the number of items
  - Generic over your random number generator

## Missing features

- No Python support
- No [Hamming distance](https://en.wikipedia.org/wiki/Hamming_distance) support
- Generally slower due to the `log(n)` lookups and non-aligned vectors due to LMDB

## Tradeoffs


Only two main parameters are needed to tune Arroy: the number of trees `n_trees` and the number of nodes to inspect during searching `search_k`.

- `n_trees` is provided during build time and affects the build time and the index size. A larger value will give more accurate results but larger indexes.
- `search_k` is provided in runtime and affects the search performance. A larger value will give more accurate results but will take a longer time to return.

If `search_k` is not provided, it will default to `n * n_trees` where `n` is the number of approximate nearest neighbors. Otherwise, `search_k` and `n_trees` are roughly independent, i.e., the value of `n_trees` will not affect search time if `search_k` is held constant and vice versa. Basically, it's recommended to set `n_trees` as large as possible given the amount of memory you can afford, and it's recommended to set `search_k` as large as possible given the time constraints you have for the queries.

## How does it work

Using [random projections](http://en.wikipedia.org/wiki/Locality-sensitive_hashing#Random_projection) and by building up a tree. At every intermediate node in the tree, a random hyperplane is chosen, which divides the space into two subspaces. This hyperplane is determined by sampling two points from the subset and taking the hyperplane equidistant from them.

We do this k times so that we get a forest of trees. k has to be tuned to your needs by looking at what tradeoff you have between precision and performance.

Dot Product distance (originally contributed by [@psobot](https://github.com/psobot) and [@pkorobov](https://github.com/pkorobov)) reduces the provided vectors from dot (or "inner-product") space to a more query-friendly cosine space using [a method by Bachrach et al., at Microsoft Research, published in 2014](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/XboxInnerProduct.pdf).


## Benchmarks

The benchmarks are available [in another repository](https://github.com/meilisearch/vector-store-relevancy-benchmark).
It shows the performances of arroy in terms of recall, disk size usage, search and indexing performances with different parameters compared to other competitors.

## Source code

It's all written in Rust and based on LMDB without a handful of ugly optimizations for performance and memory usage. You have been warned :)

The code should support Windows, thanks to LMDB and the Rust programming language.

## Big thanks to the open-source community

- Thanks to [Qdrant](https://qdrant.tech/) for their SIMD distances functions
- Thanks to Spotify for the original idea of [Annoy](https://github.com/spotify/annoy/)


[1]: https://en.wikipedia.org/wiki/Nearest_neighbor_search#Approximate_nearest_neighbor
[2]: https://github.com/spotify/annoy/#full-python-api
[3]: https://github.com/spotify/annoy/issues/96
