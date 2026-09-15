.PHONY: build server adminsrv adminctl wfctl syncer test tidy run-server run-adminsrv migrate clean

build: server adminsrv adminctl wfctl syncer

server:
	go build -o bin/server ./cmd/server

adminsrv:
	go build -o bin/adminsrv ./cmd/adminsrv

adminctl:
	go build -o bin/adminctl ./cmd/adminctl

# 数据子系统运维 CLI（直读数据库，随 deploy 上传到服务器）
syncer:
	go build -o bin/syncer ./cmd/syncer

# 定义面客户端（操作员 / agent 本地使用，走管理后台 API；deploy 不上传）
wfctl:
	go build -o bin/wfctl ./cmd/wfctl

test:
	go test ./...

tidy:
	go mod tidy

run-server: server
	./bin/server

run-adminsrv: adminsrv
	./bin/adminsrv

# 迁移到最新（首次初始化）
migrate: adminctl
	./bin/adminctl migrate up

clean:
	rm -rf bin
