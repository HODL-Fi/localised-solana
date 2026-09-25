# Switchboard Crossbar, deployable to Railway.
#
# Why we self-host at all: the hosted crossbar.switchboard.xyz returns the hash of an EMPTY payload
# from /store, so a feed created against it can never resolve and every crank dies with
# ORACLE_UNAVAILABLE. See .devnet/sb/FINDINGS.md. A local container fixed that; this makes it a URL
# instead of something that has to be running on a laptop.
#
# Pinned by digest, not :latest. This is infrastructure the feeds resolve through — a silent upstream
# retag would change how prices reach the program, which is not something to discover from a failing
# borrow.
FROM switchboardlabs/crossbar@sha256:4aad27ab0d7f4c0b8352cd3a5763a410afde323288b0dd46789f7bc58bb275b4

# The upstream entrypoint hard-codes `PORT=8080 bun run`, which makes Railway's injected $PORT a
# no-op and leaves its HTTP proxy pointing at a port nothing listens on. This is the same script
# with the same restart supervisor, honouring $PORT and defaulting to 8080 so it still behaves
# identically under plain `docker run`.
COPY entrypoint.sh /railway-entrypoint.sh
RUN chmod +x /railway-entrypoint.sh
ENTRYPOINT ["/railway-entrypoint.sh"]
