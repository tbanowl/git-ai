package com.cxmt.qms.report.waferqg.admin.controller;

import com.cxmt.qms.report.waferqg.business.IWaferQGIndexService;
import com.cxmt.qms.report.waferqg.business.IWaferQGReportService;
import com.cxmt.qms.report.waferqg.domain.dto.WaferQGChartDataQueryDTO;
import com.cxmt.qms.report.waferqg.domain.dto.WaferQGIndexDetailQueryDTO;
import com.cxmt.qms.report.waferqg.domain.dto.WaferQGRawQueryDTO;
import com.cxmt.qms.report.waferqg.domain.vo.*;
import com.cxmt.qms.report.waferqg.business.INotifySendMessageService;
import com.cxmt.rpt.framework.common.core.domain.AjaxResult;
import com.cxmt.rpt.framework.content.business.controller.BaseController;
import io.swagger.v3.oas.annotations.Operation;
import io.swagger.v3.oas.annotations.Parameter;
import io.swagger.v3.oas.annotations.tags.Tag;
import lombok.extern.slf4j.Slf4j;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.web.bind.annotation.*;

import java.util.List;
import java.util.concurrent.ExecutionException;

/**
 * @Author vendor.cno.zhongx04
 * @create 2024/6/4 10:42
 */
@RestController
@Tag(name = " WaferQGReport接口")
@Slf4j
@RequestMapping("/WaferQGReport")
public class WaferQGReportController extends BaseController {

    @Autowired
    private IWaferQGReportService waferQGReportService;

    @Autowired
    private IWaferQGIndexService waferQGIndexService;

    @Autowired
    private INotifySendMessageService iNotifySendMessageService;

    /**
     * SWR-N 查询 wafer qg char图的信息 两个饼图 以及 柱状图信息
     *
     * @param waferQgChartDataQueryDTO 查询条件
     * @return
     */
    @Operation(summary = "查询QGReport Is SWR-N")
    @PostMapping("/selectWaferQGCharData")
    public AjaxResult<WaferQGChartDataVO> selectWaferQGCharData(@RequestBody WaferQGChartDataQueryDTO waferQgChartDataQueryDTO) {
        WaferQGChartDataVO waferQGChartDataVO = waferQGReportService.selectWaferQGCharData(waferQgChartDataQueryDTO);
        return success(waferQGChartDataVO);
    }

    /**
     * SWR-Y 查询 wafer qg char图的信息 饼图 以及 柱状图信息
     *
     * @param waferQgChartDataQueryDTO 查询条件
     * @return
     */
    @Operation(summary = "查询QGReport Is SWR-Y")
    @PostMapping("/selectWaferQGSwrCharData")
    public AjaxResult<WaferQGSwrCharDataVO> selectWaferQGSwrCharData(@RequestBody WaferQGChartDataQueryDTO waferQgChartDataQueryDTO) {
        WaferQGSwrCharDataVO waferQGSwrCharDataVO = waferQGReportService.selectWaferQGSwrCharData(waferQgChartDataQueryDTO);
        return success(waferQGSwrCharDataVO);
    }

    /**
     * 饼图点击后详细信息
     *
     * @param waferQGRawQueryDTO
     * @return
     */
    @Operation(summary = "查询QGRaw详细信息")
    @PostMapping("/selectWaferDGRawGrid")
    public AjaxResult<WaferQGRawStatisticVO> selectWaferQGRawGrid(@RequestBody WaferQGRawQueryDTO waferQGRawQueryDTO) {
        WaferQGRawStatisticVO waferQGRaw = waferQGReportService.selectWaferQGRawGrid(waferQGRawQueryDTO);
        return success(waferQGRaw);
    }


    @Operation(summary = "查询QGRawIndexData详细信息")
    @PostMapping("/selectWaferQGIndexData")
    public AjaxResult<WaferQGIndexVO> selectWaferQGIndexData(@RequestBody WaferQGChartDataQueryDTO waferQgChartDataQueryDTO) {
        WaferQGIndexVO waferQGIndexVoList = waferQGIndexService.selectWaferQGIndexData(waferQgChartDataQueryDTO);
        return AjaxResult.success(waferQGIndexVoList);
    }

    @Operation(summary = "查询QGRawIndexDetailData详细信息")
    @PostMapping("/selectWaferQGIndexDetailData")
    public AjaxResult<WaferQGIndexVO> selectWaferQGIndexDetailData(@RequestBody WaferQGIndexDetailQueryDTO waferQGIndexDetailQueryDTO) {
        WaferQGIndexVO waferQGIndexVoList = waferQGIndexService.selectWaferQGIndexDetailData(waferQGIndexDetailQueryDTO);
        return AjaxResult.success(waferQGIndexVoList);
    }

    @Operation(summary = "根据fab查询产品")
    @GetMapping("/selectProdByFab")
    @Parameter(name = "fab", description = "工厂名称")
    @Parameter(name = "type", description = "NORMAL/GD")
    public AjaxResult<List<CommonVO>> selectProdByFab(String fab, String type) {
        List<CommonVO> commonVO = waferQGReportService.selectProdByFab(fab, type);
        return AjaxResult.success(commonVO);
    }

    @Operation(summary = "打印1-10测试")
    @GetMapping("/printNumbers")
    public AjaxResult<Void> printNumbers() {
        System.out.println(1);
        System.out.println(2);
        System.out.println(3);
        System.out.println("人为修改的7");
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        System.out.println("人为新增的11");
        System.out.println("人为新增的12");
        return AjaxResult.success();
    }

    @Operation(summary = "打印1-10")
    @GetMapping("/print1To10")
    public AjaxResult<Void> print1To10() {
        System.out.println(1);
        System.out.println(2);
        System.out.println(3);
        System.out.println("人工修改的7");
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        System.out.println("新增的11");
        System.out.println("新增的12");
        return AjaxResult.success();
    }

    @Operation(summary = "打印1-10纯数字")
    @GetMapping("/print1To10Only")
    public AjaxResult<Void> print1To10Only() {
        System.out.println(1);
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "打印数字1-10")
    @GetMapping("/printNumbersFrom1To10")
    public AjaxResult<Void> printNumbersFrom1To10() {
        System.out.println(1);
        System.out.println(2);
        System.out.println(3);
        System.out.println("你好4");
        System.out.println("你好5");
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "纯数字1-10打印")
    @GetMapping("/printPureNumbers")
    public AjaxResult<Void> printPureNumbers() {
        System.out.println(1);
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "数字输出1至10")
    @GetMapping("/outputNumbers")
    public AjaxResult<Void> outputNumbers() {
        System.out.println(1);
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "输出数字1-10")
    @GetMapping("/displayNumbers")
    public AjaxResult<Void> displayNumbers() {
        System.out.println(1);
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "数字展示1到10")
    @GetMapping("/showNumbers")
    public AjaxResult<Void> showNumbers() {
        System.out.println(1);
        System.out.println(2);
        System.out.println(3);
        System.out.println("编辑了7");
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        System.out.println("新增的11");
        System.out.println("新增的12");
        return AjaxResult.success();
    }

    @Operation(summary = "数字序列1-10")
    @GetMapping("/listNumbers")
    public AjaxResult<Void> listNumbers() {
        System.out.println(1);
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "输出生成1-10数字")
    @GetMapping("/generateNumbers")
    public AjaxResult<Void> generateNumbers() {
        System.out.println(1);
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "输出纯数字1-10")
    @GetMapping("/printNumbersPure")
    public AjaxResult<Void> printNumbersPure() {
        System.out.println(1);
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "打印2-10纯数字")
    @GetMapping("/print2To10Only")
    public AjaxResult<Void> print2To10Only() {
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "输出2-10纯数字")
    @GetMapping("/output2To10Only")
    public AjaxResult<Void> output2To10Only() {
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "展示2-10纯数字")
    @GetMapping("/display2To10Only")
    public AjaxResult<Void> display2To10Only() {
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "打印2-10数字")
    @GetMapping("/print2To10Numbers")
    public AjaxResult<Void> print2To10Numbers() {
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "显示2-10纯数字")
    @GetMapping("/show2To10Only")
    public AjaxResult<Void> show2To10Only() {
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "输出数字2到10")
    @GetMapping("/outputNumbers2To10")
    public AjaxResult<Void> outputNumbers2To10() {
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "显示数字2-10")
    @GetMapping("/displayNumbers2To10")
    public AjaxResult<Void> displayNumbers2To10() {
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "列出数字2-10")
    @GetMapping("/listNumbers2To10")
    public AjaxResult<Void> listNumbers2To10() {
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

    @Operation(summary = "生成数字2-10")
    @GetMapping("/generateNumbers2To10")
    public AjaxResult<Void> generateNumbers2To10() {
        System.out.println(2);
        System.out.println(3);
        System.out.println(4);
        System.out.println(5);
        System.out.println(6);
        System.out.println(7);
        System.out.println(8);
        System.out.println(9);
        System.out.println(10);
        return AjaxResult.success();
    }

}