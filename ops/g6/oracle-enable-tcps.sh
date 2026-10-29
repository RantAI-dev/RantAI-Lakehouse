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

    # Attempt 1 (kept in the log as evidence): orapki is in the image, but it needs a Java runtime that
    # the slim image does not ship ("ERROR: No Java"), so it cannot build a wallet.
    if command -v orapki >/dev/null 2>&1; then
        log "orapki: $(command -v orapki); a wallet from it: $(orapki wallet create -wallet /tmp/probe-wallet -pwd Probe1234567 -auto_login 2>&1 | head -1)"
    fi

    # Attempt 2: a PKCS#12 wallet made with openssl (`ewallet.p12`), with the generated password and then with an
    # empty one, each offered to the listener as it is. Each try rewrites listener.ora and restarts the listener
    # and logs what it said; the first that listens on 2484 stays.
    ADMIN="${TNS_ADMIN:-$ORACLE_HOME/network/admin}"
    WALLET=/opt/oracle/tcps-wallet
    mkdir -p "$ADMIN"
    # The same text, byte for byte, that the first attempt's listener accepted (it started on 1521 and only
    # refused the missing wallet); the two attempts after it built the file with `echo` and the listener
    # answered TNS-01150 / NL-00303 for every one, the plain fallback included, so what they were trying was
    # never tested.
    write_listener() {
        if [ "$1" = yes ]; then
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
        else
            cat > "$ADMIN/listener.ora" <<LISTENER
LISTENER =
  (DESCRIPTION_LIST =
    (DESCRIPTION =
      (ADDRESS = (PROTOCOL = TCP)(HOST = 0.0.0.0)(PORT = 1521))
    )
  )
LISTENER
        fi
    }
    try_wallet() {
        local label="$1" p12="$2"
        rm -rf "$WALLET" && mkdir -p "$WALLET" && chmod 700 "$WALLET"
        cp "$p12" "$WALLET/ewallet.p12"
        write_listener yes
        lsnrctl stop >/dev/null 2>&1
        lsnrctl start 2>&1 | grep -i -E "listening on|error|TNS-|ORA-" | sed "s/^/[oracle-tcps] $label: /"
        lsnrctl status 2>&1 | grep -c -i "PROTOCOL=tcps" | sed "s/^/[oracle-tcps] $label: tcps endpoints listed: /"
        lsnrctl status 2>&1 | grep -qi "PROTOCOL=tcps"
    }
    if try_wallet "password p12" /etc/oracle-tls/oracle.p12 || try_wallet "empty-password p12" /etc/oracle-tls/oracle-nopw.p12; then
        log "the listener has a TCPS endpoint"
    else
        log "no wallet the listener accepts: back to the plain listener"
        write_listener no
        lsnrctl stop >/dev/null 2>&1
        lsnrctl start 2>&1 | grep -i -E "listening on|error|TNS-" | sed 's/^/[oracle-tcps] plain: /'
    fi
    echo "alter system register;" | sqlplus -s / as sysdba 2>&1 | sed 's/^/[oracle-tcps] register: /'
    sleep 5
    lsnrctl status 2>&1 | grep -i -E "tcps|2484|FREEPDB1|error|TNS-" | sed 's/^/[oracle-tcps] status: /'
    log "done"
)
true
