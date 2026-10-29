#!/bin/bash
# ops/g6/oracle-enable-tcps.sh -- SRC-4 slice 1 review SHOULD-FIX 2: give the
# fixture's own listener a TCPS endpoint (2484) with a server wallet, so the
# gate proves the product's `tcps` dial against Oracle's own TLS and not only
# against a terminator in front of it.
#
# Mounted into /container-entrypoint-startdb.d/ of the pinned gvenzl image,
# which runs it once the database is up. The certificate, its key and a
# throwaway password come from `oracle-g6-cert-init` (mounted at
# /etc/oracle-tls); nothing certificate-shaped is committed. Every step logs
# its own outcome (`[oracle-tcps] ...`) and the script never fails the
# container: when the image cannot do a step the plain listener on 1521 is
# untouched and the log says which step it was, so a failed attempt is read from
# the compose logs and not guessed.
(
    set +e
    log() { echo "[oracle-tcps] $*"; }

    if ! command -v orapki >/dev/null 2>&1; then
        log "orapki is not in this image: no wallet can be built"
        exit 0
    fi
    log "orapki: $(command -v orapki)"

    WALLET=/opt/oracle/tcps-wallet
    P12_PASSWORD="$(cat /etc/oracle-tls/p12.pwd)"
    mkdir -p "$WALLET" && chmod 700 "$WALLET"

    orapki wallet create -wallet "$WALLET" -pwd "$P12_PASSWORD" -auto_login 2>&1 | sed 's/^/[oracle-tcps] create: /'
    orapki wallet import_pkcs12 -wallet "$WALLET" -pwd "$P12_PASSWORD" \
        -pkcs12file /etc/oracle-tls/oracle.p12 -pkcs12pwd "$P12_PASSWORD" 2>&1 | sed 's/^/[oracle-tcps] import: /'
    orapki wallet display -wallet "$WALLET" -pwd "$P12_PASSWORD" 2>&1 | sed 's/^/[oracle-tcps] display: /'
    ls -l "$WALLET" | sed 's/^/[oracle-tcps] wallet: /'

    ADMIN="${TNS_ADMIN:-$ORACLE_HOME/network/admin}"
    mkdir -p "$ADMIN"
    cat > "$ADMIN/listener.ora" <<LISTENER
LISTENER =
  (DESCRIPTION_LIST =
    (DESCRIPTION =
      (ADDRESS = (PROTOCOL = TCP)(HOST = 0.0.0.0)(PORT = 1521))
      (ADDRESS = (PROTOCOL = TCPS)(HOST = 0.0.0.0)(PORT = 2484))
    )
  )
WALLET_LOCATION =
  (SOURCE = (METHOD = FILE)(METHOD_DATA = (DIRECTORY = $WALLET)))
SSL_CLIENT_AUTHENTICATION = FALSE
LISTENER
    cat >> "$ADMIN/sqlnet.ora" <<SQLNET
WALLET_LOCATION =
  (SOURCE = (METHOD = FILE)(METHOD_DATA = (DIRECTORY = $WALLET)))
SSL_CLIENT_AUTHENTICATION = FALSE
SQLNET
    log "listener.ora and sqlnet.ora written in $ADMIN"

    lsnrctl stop 2>&1 | sed 's/^/[oracle-tcps] stop: /'
    lsnrctl start 2>&1 | sed 's/^/[oracle-tcps] start: /'
    echo "alter system register;" | sqlplus -s / as sysdba 2>&1 | sed 's/^/[oracle-tcps] register: /'
    sleep 5
    lsnrctl status 2>&1 | grep -i -E "tcps|2484|FREEPDB1|error|TNS-" | sed 's/^/[oracle-tcps] status: /'
    log "done"
)
true
